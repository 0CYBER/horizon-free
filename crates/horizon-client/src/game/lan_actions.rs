//! What guests do in a hosted world. The host's gameplay code is written for
//! the one local player, so a guest's action runs through it with the guest's
//! state swapped in for the duration of the call (position, inventory, game
//! mode, experience, mining and item use): digging, placing, using items and
//! blocks, attacking and the commands all behave exactly as they do for the
//! host, and every world change they cause reaches all players through the
//! block log, the level events and the entity tracker.
use super::{Game, GuestUse, Hit, Mining, Screen, lan::LanHost, respawn_anchor::RespawnConfig};
use horizon_client::{
    command::vanilla,
    containers::BlockEntity,
    entities::Experience,
    first_person::{Hand, SwingAnimation},
    items::ItemStack,
    lan::{
        guest::SPECTATOR,
        hub::TickCtx,
        menu::{Rules, Slots},
    },
    motion::{MotionEvent, MotionWorld, PlayerMotion},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_protocol::{
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::literal,
};
use horizon_world::entity_move::EntityMoveWorld;
use winit::keyboard::KeyCode;

/// Per-guest state of the gameplay code that lives outside `Guest`.
#[derive(Default)]
pub(super) struct GuestPlay {
    /// The sign this guest may write on (`SignBlockEntity.playerWhoMayEdit`).
    pub(super) sign_editing: Option<[i32; 3]>,
    mining: Option<Mining>,
    destroy_delay: u32,
    using: Option<(usize, u32)>,
    use_delay: u32,
    /// The shield's ItemCooldowns entry: an axe disabled it for this many ticks.
    pub(super) shield_cooldown: u32,
    attack_ticks: u32,
    /// The main hand's item at the guest's last tick (`Game::attack_item`).
    attack_item: Option<std::sync::Arc<str>>,
    swing: horizon_client::first_person::SwingState,
    /// The block being dug and the face the client aimed at.
    digging: Option<([i32; 3], [i32; 3])>,
    /// The server tick the digging started (ServerPlayerGameMode.destroyProgressStart).
    dig_start: u64,
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
    /// The inventory menu and the crafting table the guest crafts in.
    pub(super) crafting: super::lan_crafting::GuestCrafting,
    /// The workstation menu the guest has open (game/lan_stations.rs).
    pub(super) station: Option<super::lan_stations::GuestStation>,
    /// The mount inventory the guest has open (game/lan_mount_menu.rs).
    pub(super) mount_window: Option<super::lan_mount_menu::GuestMount>,
    /// The lectern the guest reads (game/lan_stations.rs).
    pub(super) lectern: Option<super::lan_stations::GuestLectern>,
    /// What the guest's body was given for its effects and enchantments.
    potions: super::potions::BodyPotions,
    /// `ServerPlayer.getStats` and the inventory `inventory_changed` last saw.
    pub(super) stats: super::stats::Stats,
    last_inventory: u64,
    /// `Entity.portalProcess` and `portalCooldown` (game/server_travel.rs).
    pub(super) portal: horizon_client::nether_portal::PortalState,
    /// `Player.takeXpDelay`: ticks until the next orb can be taken.
    xp_delay: u32,
    /// What last killed the guest outright (`/kill`): the death message's key.
    death_cause: Option<String>,
    /// The equipment modifiers on the guest's attributes (`apply_equipment`)
    /// and the armour and main hand they were put on for: the hub's player
    /// is a `ServerPlayer` too, whose worn armour counts
    /// (`LivingEntity.collectEquipmentChanges`).
    modifiers: Vec<(String, String)>,
    equipped: Option<Vec<Option<ItemStack>>>,
    /// The guest's cast fishing hook (`Player.fishing`).
    pub(super) hook: Option<super::fishing::Hook>,
}

/// A guest's command waiting for the hub (`LanHost::commands`).
pub(super) struct GuestCommand {
    conn: u32,
    command: String,
    permission: u8,
    /// An integrated server's: no level teleports (yet).
    integrated: bool,
}

/// The game's own state while a guest's is swapped in (`enter_guest`), and
/// the guest's records that live outside `Guest`.
pub(super) struct GuestSwap {
    conn: u32,
    i: usize,
    play: GuestPlay,
    player: Option<PlayerMotion>,
    /// The guest's eye height and the game's.
    eye: (f32, f32),
    yaw: f32,
    pitch: f32,
    selected: usize,
    mode: horizon_client::command::args::GameMode,
    creative: bool,
    xp: Experience,
    host_keys: std::collections::HashSet<KeyCode>,
    screen: Screen,
    host_sign: Option<super::sign_edit::SignEditor>,
    host_book: Option<super::books::BookScreen>,
    host_command_block: Option<super::command_blocks::CommandBlockScreen>,
    host_mount_menu: super::mount_menu::MountMenu,
    craft_size: usize,
    craft_grid: Vec<Option<ItemStack>>,
    container: Option<(i32, i32, i32)>,
    mining: Option<Mining>,
    destroy_delay: u32,
    using: Option<(usize, u32)>,
    use_delay: u32,
    pub(super) shield_cooldown: u32,
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
    death_cause: Option<String>,
    /// The guest whose turn this one nests in, if any.
    acting: (Option<u32>, Option<String>, Option<u128>),
    /// The packets queued for that guest.
    packets: Vec<play::Clientbound>,
    /// The host's count of swings started, kept while the guest acts.
    swings: u32,
    /// How many projectiles there were: those added since are the guest's.
    projectiles: usize,
    /// The host's equipment modifiers.
    modifiers: Vec<(String, String)>,
}

impl GuestPlay {
    /// The item in use: its slot and for how many ticks.
    pub(super) fn using(&self) -> Option<(usize, u32)> {
        self.using
    }
}

struct ItemRules<'a>(&'a horizon_client::items::Items);
impl Rules for ItemRules<'_> {
    fn items(&self) -> &horizon_client::items::Items {
        self.0
    }
    fn max_stack(&self, item: &str) -> u32 {
        self.0.get(item).map_or(64, |i| i.max_stack)
    }
    fn armor_index(&self, item: &str) -> Option<usize> {
        match self.0.get(item)?.equip_slot.as_deref()? {
            "head" => Some(0),
            "chest" => Some(1),
            "legs" => Some(2),
            "feet" => Some(3),
            _ => None,
        }
    }
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

impl Game<'_> {
    /// Run `f` as the guest `conn`: the host's player state is swapped for theirs.
    pub(super) fn as_guest<R>(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let swap = self.enter_guest(host, conn)?;
        let result = f(self);
        self.leave_guest(host, swap);
        Some(result)
    }

    /// The first half of `as_guest`: the guest's state in place of the
    /// game's, which the returned swap keeps until `leave_guest`. Between the
    /// two the host may go back into `self.lan`, so that what the guest does
    /// can reach the other players (their commands, `for_player`), and a
    /// guest's turn can nest in another's: the acting guest and the packets
    /// queued for them are kept and restored too.
    pub(super) fn enter_guest(&mut self, host: &mut LanHost, conn: u32) -> Option<GuestSwap> {
        // The guest whose turn it is is in place already (their record holds
        // the game's state until their turn ends).
        if self.lan_acting == Some(conn) {
            return None;
        }
        let i = host.hub.guests.iter().position(|g| g.conn == conn)?;
        let fresh = host.hub.guests[i].motion.is_none();
        if fresh {
            let p = Vec3::new(
                host.hub.guests[i].position[0],
                host.hub.guests[i].position[1],
                host.hub.guests[i].position[2],
            );
            host.hub.guests[i].motion = PlayerMotion::new(self.motion, p).ok().map(Box::new);
            // The first body after joining burns, breathes and falls on as saved.
            if let Some(saved) = host.hub.guests[i].loaded_body.take()
                && let Some(m) = host.hub.guests[i].motion.as_mut()
            {
                saved.apply(m);
            }
            // The saved golden hearts go on the first body of a player who
            // has been here before.
            let saved = host.hub.guests[i].saved_absorption.take();
            if let (Some(m), Some(amount)) = (host.hub.guests[i].motion.as_mut(), saved) {
                let _ = m.internal_set_absorption(amount);
            }
            let kept = host.hub.guests[i].logs_kept.take();
            if let (Some(m), Some((hurts, attacks))) = (host.hub.guests[i].motion.as_mut(), kept) {
                m.hurt_log = hurts;
                m.attack_log = attacks;
            }
        }
        let mut play = host.play.remove(&conn).unwrap_or_default();
        if fresh {
            // A new body (joined, respawned, changed level): nothing on it yet.
            play.equipped = None;
        }
        let g = &mut host.hub.guests[i];
        // The guest's body.
        let Some(mut motion) = g.motion.take() else {
            host.play.insert(conn, play);
            return None;
        };
        self.potions.swap_player(&mut g.effects, &mut play.potions);
        motion.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
        motion.ground.on_ground = g.on_ground;
        motion.yaw = g.yaw;
        motion.pitch = g.pitch;
        motion.sprinting = g.sprinting;
        motion.may_fly = g.creative() || g.game_mode == SPECTATOR;
        motion.flying = g.flying;
        motion.invulnerable = motion.may_fly;
        let _ = motion.set_health(g.health, false);
        motion.health.food.food = g.food;
        motion.health.food.saturation = g.saturation;
        let mut player = Some(*motion);
        std::mem::swap(&mut self.player, &mut player);
        let eye = (g.eye_height() as f32, self.eye);
        self.eye = eye.0;
        let (yaw, pitch) = (self.yaw, self.pitch);
        self.yaw = g.yaw;
        self.pitch = g.pitch;
        std::mem::swap(&mut self.inventory, &mut g.inventory);
        std::mem::swap(&mut self.armor, &mut g.armor);
        std::mem::swap(&mut self.offhand, &mut g.offhand);
        std::mem::swap(&mut self.carried, &mut g.carried);
        let selected = std::mem::replace(&mut self.selected, g.selected);
        let (mode, creative) = (self.mode, self.creative);
        self.set_mode_raw(g.game_mode);
        let xp = std::mem::replace(
            &mut self.experience,
            Experience {
                level: g.xp_level,
                progress: g.xp_progress,
                total: g.xp_total,
            },
        );
        // The host's held keys stay theirs: the guest holds only sneak, and a
        // screen the guest opens (`keys.clear()`) leaves the host walking.
        let host_keys = std::mem::take(&mut self.keys);
        if g.sneaking {
            self.keys.insert(KeyCode::ShiftLeft);
        }
        // The window's screens are the host's: a sign, book, command block or
        // mount menu the guest's action opens is not drawn over the host's
        // world (#687). The guest's client opens its own.
        let screen = std::mem::replace(&mut self.screen, Screen::Playing);
        let host_sign = self.sign_edit.take();
        let host_book = self.book.take();
        let host_command_block = self.command_block_screen.take();
        let host_mount_menu = std::mem::take(&mut self.mount_menu);
        // The host's crafting grid stays theirs; a crafting table the guest
        // opens shows as the 3x3 size.
        let craft_size = std::mem::replace(&mut self.craft_size, 2);
        let craft_grid = std::mem::replace(&mut self.craft_grid, vec![None; 9]);
        let container = self.container.take();
        let mining = std::mem::replace(&mut self.mining, play.mining.take());
        let destroy_delay = std::mem::replace(&mut self.destroy_delay, play.destroy_delay);
        let using = std::mem::replace(&mut self.using, play.using.take());
        let use_delay = std::mem::replace(&mut self.use_delay, play.use_delay);
        let shield_cooldown = std::mem::replace(&mut self.shield_cooldown, play.shield_cooldown);
        let attack_ticks = std::mem::replace(&mut self.attack_ticks, play.attack_ticks);
        let attack_item = std::mem::replace(&mut self.attack_item, play.attack_item.take());
        let swing = std::mem::replace(&mut self.swing, play.swing);
        std::mem::swap(&mut self.mount, &mut play.mount);
        let (left, right) = (self.left, self.right);
        self.left = play.digging.is_some();
        self.right = play.using.is_some() || self.using.is_some();
        // The guest's vehicle (Entity.vehicle) for its turn.
        let riding = std::mem::replace(&mut self.riding, g.vehicle);
        let host_station = self.station.take();
        // The guest's bed, sleep counter and respawn point; the host's
        // respawn point, anchor and action bar stay theirs.
        let sleeping = std::mem::replace(&mut self.sleeping, g.bed);
        let sleep_timer = std::mem::replace(&mut self.sleep_timer, g.sleep_timer);
        let spawn = std::mem::replace(
            &mut self.spawn,
            g.spawn.map(|s| Vec3::new(s[0], s[1], s[2])),
        );
        let respawn = (self.respawn_config.take(), self.level.spawn);
        self.respawn_config = g.respawn_elsewhere.as_ref().map(RespawnConfig::from_saved);
        let message = self.message.take();
        // The guest's own ender inventory; one the host has open goes back
        // into the host's while the guest acts.
        let host_ender = self.ender_open;
        self.close_ender_chest();
        std::mem::swap(&mut self.ender_items, &mut g.ender_items);
        // The guest's advancements, recipe book and statistics.
        self.advancement_tree();
        std::mem::swap(&mut self.adv.progress, &mut g.advancements);
        std::mem::swap(&mut self.adv.changed, &mut g.advancements_changed);
        std::mem::swap(&mut self.adv.unlocked, &mut g.recipes_unlocked);
        std::mem::swap(&mut self.level.recipe_book, &mut g.recipe_book);
        std::mem::swap(&mut self.stats, &mut play.stats);
        std::mem::swap(&mut self.hook, &mut play.hook);
        let adv_loaded = std::mem::replace(&mut self.adv.loaded, true);
        let last_inventory = std::mem::replace(&mut self.adv.last_inventory, play.last_inventory);
        let death_cause = std::mem::replace(&mut self.ui.death_cause, play.death_cause.take());
        let acting = (
            self.lan_acting.replace(conn),
            self.lan_acting_name.replace(g.name.clone()),
            self.lan_acting_uuid.replace(g.uuid),
        );
        let packets = std::mem::take(&mut self.guest_packets);
        let swings = self.swings_started;
        let projectiles = self.creatures.projectiles.len();
        let modifiers = std::mem::replace(
            &mut self.applied_modifiers,
            std::mem::take(&mut play.modifiers),
        );
        // Its worn armour on its attributes, when it changed.
        let mut equipped: Vec<Option<ItemStack>> = self.armor.to_vec();
        equipped.push(self.inventory.get(self.selected).cloned().flatten());
        if play.equipped.as_ref() != Some(&equipped) {
            self.apply_equipment();
            play.equipped = Some(equipped);
        }
        self.update_totem_ready();
        Some(GuestSwap {
            conn,
            i,
            play,
            player,
            eye,
            yaw,
            pitch,
            selected,
            mode,
            creative,
            xp,
            host_keys,
            screen,
            host_sign,
            host_book,
            host_command_block,
            host_mount_menu,
            craft_size,
            craft_grid,
            container,
            mining,
            destroy_delay,
            using,
            use_delay,
            shield_cooldown,
            attack_ticks,
            attack_item,
            swing,
            left,
            right,
            riding,
            host_station,
            sleeping,
            sleep_timer,
            spawn,
            respawn,
            message,
            host_ender,
            adv_loaded,
            last_inventory,
            death_cause,
            acting,
            packets,
            swings,
            projectiles,
            modifiers,
        })
    }

    /// The second half of `as_guest`: the guest's state back into their
    /// record, the game's back in place, and what the turn changed sent.
    pub(super) fn leave_guest(&mut self, host: &mut LanHost, swap: GuestSwap) {
        // A totem that saved the guest this turn (its hurt set it off).
        let totem = self.apply_totem();
        let GuestSwap {
            conn,
            i,
            mut play,
            mut player,
            eye,
            yaw,
            pitch,
            selected,
            mode,
            creative,
            xp,
            host_keys,
            screen,
            host_sign,
            host_book,
            host_command_block,
            host_mount_menu,
            craft_size,
            craft_grid,
            container,
            mining,
            destroy_delay,
            using,
            use_delay,
            shield_cooldown,
            attack_ticks,
            attack_item,
            swing,
            left,
            right,
            riding,
            host_station,
            sleeping,
            sleep_timer,
            spawn,
            respawn,
            message,
            host_ender,
            adv_loaded,
            last_inventory,
            death_cause,
            acting,
            packets,
            swings,
            projectiles,
            modifiers,
        } = swap;
        play.modifiers = std::mem::replace(&mut self.applied_modifiers, modifiers);
        // What the guest shot is theirs (Projectile.setOwner).
        for p in self.creatures.projectiles.iter_mut().skip(projectiles) {
            if p.owner == Some(horizon_client::mobs::Who::Player) && p.guest.is_none() {
                p.guest = Some(conn);
            }
        }
        // The guest's swings are theirs: the host's own count stays the host's.
        play.swings += self.swings_started.wrapping_sub(swings);
        self.swings_started = swings;
        for packet in std::mem::replace(&mut self.guest_packets, packets) {
            host.hub.send_to(conn, &packet);
        }
        (self.lan_acting, self.lan_acting_name, self.lan_acting_uuid) = acting;
        play.death_cause = std::mem::replace(&mut self.ui.death_cause, death_cause);
        // An ender chest the guest opened: the menu shows their inventory
        // itself (lan_open_container), not a stand-in block entity.
        self.close_ender_chest();

        // What the action opened: handed to the guest instead of the host.
        let opened = self.container.take();
        let opened_table = std::mem::replace(&mut self.craft_size, craft_size) == 3;
        self.craft_grid = craft_grid;
        let opened_station = std::mem::replace(&mut self.station, host_station);
        let bed = std::mem::replace(&mut self.sleeping, sleeping);
        let guest_sleep_timer = std::mem::replace(&mut self.sleep_timer, sleep_timer);
        let guest_spawn = std::mem::replace(&mut self.spawn, spawn).map(|s| [s.x, s.y, s.z]);
        let guest_respawn = std::mem::replace(&mut self.respawn_config, respawn.0);
        self.level.spawn = respawn.1;
        // ServerPlayer.displayClientMessage(_, true): the guest's action bar.
        let guest_message = std::mem::replace(&mut self.message, message);
        let guest_vehicle = std::mem::replace(&mut self.riding, riding);
        self.left = left;
        self.right = right;
        play.mining = std::mem::replace(&mut self.mining, mining);
        play.destroy_delay = std::mem::replace(&mut self.destroy_delay, destroy_delay);
        play.using = std::mem::replace(&mut self.using, using);
        play.use_delay = std::mem::replace(&mut self.use_delay, use_delay);
        play.shield_cooldown = std::mem::replace(&mut self.shield_cooldown, shield_cooldown);
        play.attack_ticks = std::mem::replace(&mut self.attack_ticks, attack_ticks);
        play.attack_item = std::mem::replace(&mut self.attack_item, attack_item);
        play.swing = std::mem::replace(&mut self.swing, swing);
        std::mem::swap(&mut self.mount, &mut play.mount);
        self.container = container;
        self.screen = screen;
        // A sign the guest's placement opened (SignBlock.openTextEdit).
        let guest_sign = std::mem::replace(&mut self.sign_edit, host_sign);
        // A lectern the guest's use opened (LecternBlock.openScreen).
        let guest_lectern = match std::mem::replace(&mut self.book, host_book) {
            Some(super::books::BookScreen::View {
                lectern: Some(key), ..
            }) => Some(key),
            _ => None,
        };
        self.command_block_screen = host_command_block;
        // A mount inventory the guest's action opened (shift + use, or the
        // inventory key while riding).
        let guest_mount = std::mem::replace(&mut self.mount_menu, host_mount_menu).opened_mob();
        self.keys = host_keys;
        let g = &mut host.hub.guests[i];
        let left_vehicle = g.vehicle.filter(|_| guest_vehicle.is_none());
        if guest_vehicle.is_none() {
            g.vehicle_good = None;
        }
        g.vehicle = guest_vehicle;
        let fell_asleep = bed.is_some() && g.bed.is_none();
        (g.bed, g.sleep_timer, g.spawn) = (bed, guest_sleep_timer, guest_spawn);
        g.respawn_elsewhere = guest_respawn.map(|c| c.saved());
        g.sleeping = g.bed.is_some();
        if let Some((text, _)) = guest_message {
            g.send(&C::SystemChat(play::SystemChat {
                content: literal(&text),
                overlay: true,
            }));
        }
        std::mem::swap(&mut self.ender_items, &mut g.ender_items);
        std::mem::swap(&mut self.adv.progress, &mut g.advancements);
        std::mem::swap(&mut self.adv.changed, &mut g.advancements_changed);
        std::mem::swap(&mut self.adv.unlocked, &mut g.recipes_unlocked);
        std::mem::swap(&mut self.level.recipe_book, &mut g.recipe_book);
        std::mem::swap(&mut self.stats, &mut play.stats);
        std::mem::swap(&mut self.hook, &mut play.hook);
        self.adv.loaded = adv_loaded;
        play.last_inventory = std::mem::replace(&mut self.adv.last_inventory, last_inventory);
        let stat_events = std::mem::take(&mut play.stats.events);
        let name = g.name.clone();
        self.potions.swap_player(&mut g.effects, &mut play.potions);
        if let Some(key) = host_ender {
            self.world
                .block_entities
                .insert(key, BlockEntity::Chest(self.ender_items.clone()));
            self.ender_open = Some(key);
        }
        let new_xp = std::mem::replace(&mut self.experience, xp);
        g.xp_level = new_xp.level;
        g.xp_progress = new_xp.progress;
        g.xp_total = new_xp.total;
        let new_mode = self.mode_id();
        self.mode = mode;
        self.creative = creative;
        self.selected = std::mem::replace(&mut g.selected, selected);
        // `g.selected` now holds the host's value: swap the inventories back.
        std::mem::swap(&mut self.selected, &mut g.selected);
        std::mem::swap(&mut self.inventory, &mut g.inventory);
        std::mem::swap(&mut self.armor, &mut g.armor);
        std::mem::swap(&mut self.offhand, &mut g.offhand);
        std::mem::swap(&mut self.carried, &mut g.carried);
        // A teleport with a rotation (commands) turns the guest.
        let turned = [self.yaw, self.pitch] != [g.yaw, g.pitch];
        [g.yaw, g.pitch] = [self.yaw, self.pitch];
        self.yaw = yaw;
        self.pitch = pitch;
        self.eye = eye.1;
        std::mem::swap(&mut self.player, &mut player);
        if let Some(mut m) = player {
            g.health = m.health.health();
            g.food = m.health.food.food;
            g.saturation = m.health.food.saturation;
            // A teleport (commands, a pearl) moves the guest; a pearl's is
            // sent even to where they stand (ServerPlayer.teleport).
            let at = [m.position.x, m.position.y, m.position.z];
            // A hit with extra knockback stopped the guest's sprint
            // (Player.causeExtraKnockback's setSprinting(false)).
            if !m.sprinting {
                g.sprinting = false;
            }
            m.sprinting = false;
            let teleported = std::mem::take(&mut m.teleported);
            let moved: f64 = (0..3).map(|k| (at[k] - g.position[k]).powi(2)).sum();
            g.motion = Some(Box::new(m));
            // Entity.teleportTo takes a rider out of its vehicle.
            let left_vehicle = match g.vehicle.filter(|_| teleported) {
                Some(v) => {
                    (g.vehicle, g.vehicle_good) = (None, None);
                    Some(v)
                }
                None => left_vehicle,
            };
            // ServerPlayer.dismountTo: the client hears it is out of the
            // vehicle before the teleport beside it. A rider sits where its
            // vehicle puts it (lan_vehicles.rs), with no teleport.
            if let Some(vehicle) = left_vehicle {
                g.send(&C::SetPassengers(play::SetPassengers {
                    vehicle: vehicle as i32,
                    passengers: Vec::new(),
                }));
            }
            if g.vehicle.is_none() && (moved > 0.0001 || turned || teleported) {
                host.hub.teleport(i, Some(at));
            }
        }
        let g = &mut host.hub.guests[i];
        if let Some(head) = g.bed.filter(|_| fell_asleep) {
            // LivingEntity.startSleeping: in the bed, still.
            let at = [
                f64::from(head[0]) + 0.5,
                f64::from(head[1]) + 0.5625,
                f64::from(head[2]) + 0.5,
            ];
            g.position = at;
            if let Some(m) = g.motion.as_mut() {
                m.position = Vec3::new(at[0], at[1], at[2]);
                m.velocity = Vec3::ZERO;
            }
        }
        if new_mode != g.game_mode {
            let conn = g.conn;
            host.hub.set_game_mode(conn, new_mode);
        }
        // LivingEntity.checkTotemDeathProtection's entity event, to every
        // client around, the guest's own included; this window draws it too.
        if let Some(body) = totem {
            let id = host.hub.guests[i].entity_id;
            host.hub
                .entity_event(id, super::totem::TOTEM_EVENT, body.at);
            if self.player.is_some() {
                self.totem_event(body, false);
            }
        }
        host.play.insert(conn, play);
        self.stat_criteria_changed(&name, stat_events);
        if let Some(pos) = opened
            && !self.lan_open_container(host, conn, pos)
            && !host.hub.guests[i].spectator()
        {
            // Nothing the guest can use opened: its lid closes again.
            self.container_openers_changed(pos, false);
        }
        if let Some(station) = opened_station {
            self.lan_station_opened(host, conn, station);
        }
        if let Some(edit) = guest_sign {
            self.lan_open_sign_editor(host, conn, edit);
        }
        if let Some(key) = guest_lectern {
            self.lan_open_lectern(host, conn, key);
        }
        if opened_table {
            self.lan_open_crafting(host, conn);
        }
        if let Some(mob) = guest_mount {
            self.lan_open_mount(host, conn, mob);
        }
    }

    /// `ServerLevel.players()` in turn: the host's own player, then each
    /// guest in the level in place with their state swapped in (`as_guest`).
    /// What reaches the players around a point (a beacon's powers, a splash
    /// potion, a warden's darkness, an elder guardian's curse) lands on each
    /// of them, not only on the host. A server has no player of its own.
    /// Inside a guest's action (the hub taken) only that player is in place.
    pub(super) fn for_each_player(&mut self, mut f: impl FnMut(&mut Self)) {
        if self.player.is_some() {
            f(self);
        }
        let Some(mut host) = self.lan.take() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.as_guest(&mut host, conn, &mut f);
        }
        self.lan = Some(host);
    }

    /// The game mode fields without `set_mode`'s side effects on the host.
    fn set_mode_raw(&mut self, id: i32) {
        use horizon_client::command::args::GameMode;
        self.mode = match id {
            1 => GameMode::Creative,
            2 => GameMode::Adventure,
            3 => GameMode::Spectator,
            _ => GameMode::Survival,
        };
        self.creative = self.mode == GameMode::Creative;
    }

    /// Hand a container the guest opened over (ServerPlayer.openMenu);
    /// false when it is not one a guest can open.
    fn lan_open_container(&mut self, host: &mut LanHost, conn: u32, pos: (i32, i32, i32)) -> bool {
        let name = &self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name;
        let (menu, title, slots) = if &**name == "minecraft:ender_chest" {
            // EnderChestBlock.useWithoutItem: the player's own inventory.
            (
                "minecraft:generic_9x3",
                "container.enderchest",
                Slots::Ender,
            )
        } else if let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) {
            // AbstractFurnaceBlock.openContainer: FurnaceMenu, BlastFurnaceMenu, SmokerMenu.
            let (menu, title) = match f.kind.recipe_type() {
                "minecraft:blasting" => ("minecraft:blast_furnace", "container.blast_furnace"),
                "minecraft:smoking" => ("minecraft:smoker", "container.smoker"),
                _ => ("minecraft:furnace", "container.furnace"),
            };
            (menu, title, Slots::Block(pos))
        } else if let Some(BlockEntity::Crafter(_)) = self.world.block_entities.get(&pos) {
            // CrafterBlock.useWithoutItem: CrafterMenu over the block entity.
            (
                "minecraft:crafter_3x3",
                "container.crafter",
                Slots::Block(pos),
            )
        } else if let Some(BlockEntity::Brewing(_)) = self.world.block_entities.get(&pos) {
            // BrewingStandBlock.useWithoutItem: BrewingStandMenu.
            (
                "minecraft:brewing_stand",
                "container.brewing",
                Slots::Block(pos),
            )
        } else if let Some((other, right)) = self.chest_partner(pos) {
            // ChestBlock.combine: the right half's slots come first.
            let halves = if right {
                Slots::Double(pos, other)
            } else {
                Slots::Double(other, pos)
            };
            ("minecraft:generic_9x6", "container.chestDouble", halves)
        } else {
            let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&pos) else {
                return false;
            };
            let (menu, title) = match items.len() {
                27 => ("minecraft:generic_9x3", "container.chest"),
                9 => ("minecraft:generic_3x3", "container.dispenser"),
                5 => ("minecraft:hopper", "container.hopper"),
                _ => return false,
            };
            let title = match &**name {
                "minecraft:barrel" => "container.barrel",
                "minecraft:dropper" => "container.dropper",
                n if n.ends_with("shulker_box") => "container.shulkerBox",
                _ => title,
            };
            (menu, title, Slots::Block(pos))
        };
        let Some(items) = host
            .hub
            .guest(conn)
            .and_then(|g| slots.read(&self.world, g))
        else {
            return false;
        };
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_container_closed(host, conn);
        self.lan_station_closed(host, conn, None);
        host.hub.open_container(
            conn,
            menu,
            horizon_protocol::text::translatable(title, &[]),
            pos,
            slots,
            items,
        );
        true
    }

    /// The guest's open container is closing (`AbstractContainerMenu.removed`):
    /// it no longer counts among the openers of the lid.
    pub(super) fn guest_container_closed(&mut self, host: &mut LanHost, conn: u32) {
        if let Some(g) = host.hub.guest(conn)
            && let Some(pos) = g.menu.as_ref().and_then(|m| m.pos)
            && !g.spectator()
        {
            self.container_openers_changed(pos, false);
        }
    }

    /// A packet from a guest that the hub does not handle itself.
    pub(super) fn lan_guest_packet(&mut self, host: &mut LanHost, conn: u32, packet: S) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        // A dead player acts no more, but the death screen still respawns and chats: its
        // commands run as anyone's (ServerGamePacketListenerImpl.handleChatCommand doesn't
        // look at the player's health).
        if host.hub.guests[gi].dead
            && !matches!(
                packet,
                S::ClientCommand(_) | S::ChatCommand(_) | S::ChatCommandSigned(_)
            )
        {
            return;
        }
        let spectator = host.hub.guests[gi].spectator();
        match packet {
            S::PlayerAction(a) => match a.action {
                0 => {
                    host.hub.ack(conn, a.sequence);
                    if spectator {
                        return;
                    }
                    let face = direction(i32::from(a.direction));
                    let play = host.play.entry(conn).or_default();
                    play.digging = Some((a.pos, face));
                    play.dig_start = host.hub.ticks;
                    play.last_progress = -1;
                    // The pause between two blocks is the client's (MultiPlayerGameMode's
                    // destroyDelay); the server digs from the START it receives.
                    play.destroy_delay = 0;
                }
                1 => {}
                3 => {
                    host.hub.ack(conn, a.sequence);
                    self.guest_stop_digging(host, conn, a.pos);
                }
                2 => {
                    host.hub.ack(conn, a.sequence);
                    let play = host.play.entry(conn).or_default();
                    play.digging = None;
                    play.mining = None;
                    let state = self.world.block(a.pos[0], a.pos[1], a.pos[2]).0 as i32;
                    host.hub.send_to(
                        conn,
                        &C::BlockUpdate(play::BlockUpdate { pos: a.pos, state }),
                    );
                    self.destroy_block_progress(host.hub.guests[gi].entity_id, a.pos, -1);
                    host.hub.broadcast_near(
                        [
                            f64::from(a.pos[0]),
                            f64::from(a.pos[1]),
                            f64::from(a.pos[2]),
                        ],
                        64.0,
                        &C::BlockDestruction(play::BlockDestruction {
                            id: host.hub.guests[gi].entity_id,
                            pos: a.pos,
                            progress: 255,
                        }),
                        Some(conn),
                    );
                }
                4 | 5 => {
                    if !spectator {
                        self.as_guest(host, conn, |g| g.drop_held(a.action == 4));
                    }
                }
                // RELEASE_USE_ITEM: ServerPlayer.releaseUsingItem, with the
                // hand of the item in use acting.
                6 => {
                    self.as_guest(host, conn, |g| {
                        let Some(hand) = g.used_hand() else {
                            return;
                        };
                        g.with_hand(hand, |g| {
                            if let Some((slot, ticks)) = g.using.take()
                                && let Some(stack) = g.stack_in(slot).cloned()
                            {
                                g.release_used_item(&stack, ticks, false);
                            }
                        });
                    });
                }
                // SWAP_ITEM_WITH_OFFHAND: the hands swap and the item in use stops
                // (ServerGamePacketListenerImpl.handlePlayerAction), not for spectators.
                7 if !spectator => {
                    let g = &mut host.hub.guests[gi];
                    let slot = g.selected;
                    std::mem::swap(&mut g.inventory[slot], &mut g.offhand);
                    let (offhand, main) = (g.offhand.clone(), g.inventory[slot].clone());
                    if let Some(play) = host.play.get_mut(&conn) {
                        play.using = None;
                    }
                    // setItemInHand: the off hand, then the main hand.
                    self.as_guest(host, conn, |g| {
                        g.player_on_equip("offhand", main.as_ref(), offhand.as_ref());
                        g.player_on_equip("mainhand", offhand.as_ref(), main.as_ref());
                    });
                }
                _ => {}
            },
            S::SetCarriedItem(p) => {
                if (0..9).contains(&p.slot) {
                    host.hub.guests[gi].selected = p.slot as usize;
                    if let Some(play) = host.play.get_mut(&conn) {
                        play.using = None;
                    }
                }
            }
            // ServerGamePacketListenerImpl.handlePunch: the main hand swings
            // with the held item's attack animation (mining, attacking, a
            // click on nothing), seen by the players around, and the attack
            // strength starts again.
            S::Punch(_) => {
                self.as_guest(host, conn, |g| {
                    g.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
                    g.reset_attack_strength();
                });
            }
            S::UseItemOn(p) => {
                host.hub.ack(conn, p.sequence);
                if spectator {
                    return;
                }
                let face = direction(p.face);
                let hit = Hit {
                    pos: p.pos,
                    face,
                    point: [
                        f64::from(p.pos[0]) + f64::from(p.cursor[0]),
                        f64::from(p.pos[1]) + f64::from(p.cursor[1]),
                        f64::from(p.pos[2]) + f64::from(p.cursor[2]),
                    ],
                };
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let logged = self.world.block_log.as_ref().map_or(0, Vec::len);
                let heard = self.audio.capture.as_ref().map_or(0, Vec::len);
                let swung = self.as_guest(host, conn, |g| {
                    g.forced_hit = Some(hit);
                    g.guest_use = Some(GuestUse::OnBlock(hand));
                    let swings = g.swings_started;
                    let _ = g.use_item();
                    g.forced_hit = None;
                    g.guest_use = None;
                    g.swings_started != swings
                });
                self.lan_swing_back(host, conn, swung);
                self.guest_use_sounds(host, conn, logged, heard);
                // The client predicted a result: correct it with what really is.
                for pos in [
                    p.pos,
                    [p.pos[0] + face[0], p.pos[1] + face[1], p.pos[2] + face[2]],
                ] {
                    let state = self.world.block(pos[0], pos[1], pos[2]).0 as i32;
                    host.hub
                        .send_to(conn, &C::BlockUpdate(play::BlockUpdate { pos, state }));
                }
            }
            S::UseItem(p) => {
                host.hub.ack(conn, p.sequence);
                if spectator {
                    return;
                }
                // handleUseItem: the client's rotation (absSnapRotationTo, which
                // sends nothing back), then the item alone. What it looked at a
                // moment ago (a block, a mob) is UseItemOn's or Interact's.
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let g = &mut host.hub.guests[gi];
                (g.yaw, g.pitch) = (p.yaw, p.pitch);
                let swung = self.as_guest(host, conn, |g| {
                    g.guest_use = Some(GuestUse::Item(hand));
                    let swings = g.swings_started;
                    let _ = g.use_item();
                    g.guest_use = None;
                    g.swings_started != swings
                });
                self.lan_swing_back(host, conn, swung);
            }
            S::Attack(p) => self.guest_attack(host, conn, p.id),
            S::Interact(p) => {
                let before = host.hub.guests[gi].vehicle;
                if self
                    .look_at_entity(host, gi, p.id, Some(p.location))
                    .is_some()
                {
                    // handleInteract: the entity and that hand only; a click the
                    // entity passes comes back as UseItem.
                    let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                    let swung = self.as_guest(host, conn, |g| {
                        g.guest_use = Some(GuestUse::Entity(hand));
                        let swings = g.swings_started;
                        let _ = g.use_item();
                        g.guest_use = None;
                        g.swings_started != swings
                    });
                    self.lan_swing_back(host, conn, swung);
                    self.guest_boarded(host, conn, before);
                }
            }
            S::MoveVehicle(p) => self.guest_move_vehicle(host, conn, &p),
            S::PaddleBoat(p) => self.guest_paddle_boat(host, conn, [p.left, p.right]),
            S::SetCreativeModeSlot(p) => {
                let g = &mut host.hub.guests[gi];
                if !g.creative() || !(0..46).contains(&p.slot) {
                    return;
                }
                let item = self.item_from_network(host, &p.item);
                let slot = p.slot as usize;
                let g = &mut host.hub.guests[gi];
                let old = g.menu_slot(slot);
                g.set_menu_slot(slot, item.clone());
                // InventoryMenu's ArmorSlot and off-hand slot: setByPlayer.
                let equipment = match slot {
                    5..=8 => Some(["head", "chest", "legs", "feet"][slot - 5]),
                    45 => Some("offhand"),
                    _ => None,
                };
                if let Some(equipment) = equipment {
                    self.as_guest(host, conn, |g| {
                        g.player_on_equip(equipment, old.as_ref(), item.as_ref());
                    });
                }
            }
            S::ContainerClick(p) if self.lan_merchant_click(host, conn, &p) => {}
            S::ContainerClick(p) if self.lan_inventory_click(host, conn, &p) => {}
            S::ContainerClick(p) if self.lan_crafting_click(host, conn, &p) => {}
            S::ContainerClick(p) if self.lan_machine_click(host, conn, &p) => {}
            S::ContainerClick(p) if self.lan_station_click(host, conn, &p) => {}
            S::ContainerClick(p) if self.lan_lectern_click(host, conn, &p) => {}
            S::ContainerClick(p) if self.lan_mount_click(host, conn, &p) => {}
            // ServerboundPlayerCommandPacket OPEN_INVENTORY: the ridden
            // mount's inventory (`HasCustomInventoryScreen`).
            S::PlayerCommand(p) if p.action == 5 => {
                self.as_guest(host, conn, |g| {
                    g.open_ridden_mount_menu();
                });
            }
            S::PlayerCommand(p) if p.action == 6 => self.lan_start_gliding(host, conn),
            S::ContainerButtonClick(p) => {
                self.lan_station_button(host, conn, p.container, p.button);
            }
            S::RenameItem(p) => self.lan_station_rename(host, conn, &p.name),
            S::SignUpdate(p) => self.lan_sign_update(host, conn, p),
            S::EditBook(p) => self.lan_edit_book(host, conn, p),
            S::ContainerSlotStateChanged(p) => {
                self.lan_crafter_slot_state(host, conn, p.container, p.slot, p.enabled);
            }
            S::SetBeacon(p) => self.lan_station_beacon(host, conn, p.primary, p.secondary),
            S::SelectTrade(p) => self.lan_select_trade(host, conn, p.item),
            S::ContainerClose(p) if self.lan_merchant_closed(host, conn, Some(p.container)) => {
                self.guest_carried_back(host, gi);
            }
            S::ContainerClose(p)
                if self.lan_crafting_closed(host, conn, Some(p.container))
                    || self.lan_station_closed(host, conn, Some(p.container))
                    || self.lan_mount_closed(host, conn, Some(p.container)) =>
            {
                self.guest_carried_back(host, gi);
            }
            S::ContainerClick(p) => {
                let rules = ItemRules(self.res.items());
                let drops = host.hub.container_click(conn, &p, &mut self.world, &rules);
                self.drop_guest_items(host, gi, drops);
            }
            S::ContainerClose(p) => {
                if host.hub.guests[gi]
                    .menu
                    .as_ref()
                    .is_some_and(|m| m.window == p.container)
                {
                    self.guest_container_closed(host, conn);
                }
                host.hub.container_closed(conn, p.container);
                self.guest_carried_back(host, gi);
            }
            S::ChatCommand(c) => self.guest_chat_command(host, conn, &c.command),
            S::ChatCommandSigned(p) => self.guest_signed_command(host, conn, p),
            S::ChangeDifficulty(p) if host.hub.guests[gi].permission >= 2 => {
                if !self.level.difficulty_locked && (0..=3).contains(&p.difficulty) {
                    self.ui.difficulty = if self.level.hardcore {
                        3
                    } else {
                        p.difficulty as u8
                    };
                    self.sync_rules();
                    if self.ui.difficulty == 0 {
                        self.mobs.retain(|m| !m.hostile());
                    }
                    host.hub
                        .broadcast(&C::ChangeDifficulty(play::ChangeDifficulty {
                            difficulty: i32::from(self.ui.difficulty),
                            locked: self.level.difficulty_locked,
                        }));
                }
            }
            S::LockDifficulty(p) if host.hub.guests[gi].permission >= 2 => {
                self.level.difficulty_locked = p.locked;
                host.hub
                    .broadcast(&C::ChangeDifficulty(play::ChangeDifficulty {
                        difficulty: i32::from(self.ui.difficulty),
                        locked: self.level.difficulty_locked,
                    }));
            }
            S::ClientCommand(p) if p.action == 2 && host.hub.guests[gi].permission >= 2 => {
                host.hub.send_to(
                    conn,
                    &C::GameRuleValues(play::GameRuleValues {
                        entries: horizon_client::gamerules::RULES
                            .iter()
                            .map(|r| play::GameRuleEntry {
                                name: format!("minecraft:{}", r.name),
                                value: self.ui.rules.get(r.name).unwrap_or(r.default).to_string(),
                            })
                            .collect(),
                    }),
                );
            }
            S::SetGameRule(p) if host.hub.guests[gi].permission >= 2 => {
                for entry in p.entries {
                    if self.ui.rules.set_string(&entry.name, &entry.value) {
                        let message = horizon_client::text::Text::tr(
                            "commands.gamerule.set",
                            &[&entry.name, &entry.value],
                        );
                        let content =
                            horizon_client::lan::data::component_nbt(&message.component());
                        for g in &host.hub.guests {
                            if g.permission >= 2 {
                                g.send(&C::SystemChat(play::SystemChat {
                                    content: content.clone(),
                                    overlay: false,
                                }));
                            }
                        }
                        if self.options.dedicated.is_none() && self.allow_commands() {
                            self.system_message(message);
                        }
                    }
                }
                self.sync_rules();
            }
            S::ChangeGameMode(p) if host.hub.guests[gi].permission >= 2 => {
                if let Some(mode) = usize::try_from(p.mode)
                    .ok()
                    .and_then(|index| horizon_client::command::args::GameMode::ALL.get(index))
                {
                    let lines = self.as_guest(host, conn, |g| {
                        g.chat_capture = Some(Vec::new());
                        g.change_game_mode(*mode);
                        g.chat_capture
                            .take()
                            .unwrap_or_default()
                            .iter()
                            .map(|t| horizon_client::lan::data::component_nbt(&t.component()))
                            .collect::<Vec<_>>()
                    });
                    for content in lines.unwrap_or_default() {
                        host.hub.send_to(
                            conn,
                            &C::SystemChat(play::SystemChat {
                                content,
                                overlay: false,
                            }),
                        );
                    }
                    self.send_admin_feedback(host, Some(conn));
                }
            }
            S::Unknown(id, payload)
                if id == horizon_protocol::ids::play::serverbound::COMMAND_SUGGESTION =>
            {
                self.guest_suggestions(host, conn, &payload);
            }
            S::PlaceRecipe(p) => self.lan_place_recipe(host, conn, &p),
            S::ClientCommand(c) if c.action == 0 => self.guest_respawn(host, conn),
            // ServerStatsCounter.sendStats.
            S::ClientCommand(c) if c.action == 1 => {
                let stats = host
                    .play
                    .get_mut(&conn)
                    .map(|p| p.stats.take_dirty())
                    .unwrap_or_default();
                host.hub.send_stats(conn, &stats);
            }
            S::PickItemFromBlock(p) => {
                let state = self.world.block(p.pos[0], p.pos[1], p.pos[2]);
                let item = self.res.facts().get(state).item.clone();
                let g = &mut host.hub.guests[gi];
                if g.creative() && &*item != "minecraft:air" {
                    let slot = g.selected;
                    g.inventory[slot] = Some(ItemStack::new(&item, 1));
                }
            }
            _ => {}
        }
    }

    fn item_from_network(
        &self,
        host: &LanHost,
        stack: &Option<horizon_protocol::item::Stack>,
    ) -> Option<ItemStack> {
        use horizon_protocol::item::{Component, component};
        let s = stack.as_ref()?;
        let name = host
            .hub
            .item_name(s.item)
            .unwrap_or_else(|| "minecraft:air".into());
        let mut out = ItemStack::new(&name, s.count.max(1) as u32);
        out.damage = s.damage().max(0) as u32;
        if let Some(Component::Enchantments(list)) = s.component(component::ENCHANTMENTS) {
            for (id, level) in list {
                if let Some(e) = host.hub.enchantment_name(*id) {
                    out.enchant(&e, *level as u32);
                }
            }
        }
        Some(out)
    }

    /// `AbstractContainerMenu.removed`: what a closing player held on the
    /// cursor goes back into their inventory (`placeItemBackInInventory`,
    /// dropped only if it doesn't fit), and their client's cursor empties
    /// with the inventory it is sent.
    fn guest_carried_back(&mut self, host: &mut LanHost, gi: usize) {
        if let Some(c) = host.hub.guests[gi].carried.take() {
            self.lan_place_back(host, gi, vec![c]);
            host.hub.send_inventory(gi);
        }
    }

    pub(super) fn drop_guest_items(&mut self, host: &LanHost, gi: usize, stacks: Vec<ItemStack>) {
        let g = &host.hub.guests[gi];
        let eye = [
            g.position[0],
            g.position[1] + g.eye_height() - 0.3,
            g.position[2],
        ];
        let (yaw, pitch) = (
            f64::from(g.yaw).to_radians(),
            f64::from(g.pitch).to_radians(),
        );
        for stack in stacks {
            let v = [
                -yaw.sin() * pitch.cos() * 0.3,
                -pitch.sin() * 0.3 + 0.1,
                yaw.cos() * pitch.cos() * 0.3,
            ];
            self.spawn_item(stack, eye, v, 40);
        }
    }

    /// Aim the swapped-in player at an entity; returns where it aims and the
    /// eye. A boat, minecart, armor stand, item frame or painting is aimed at
    /// the point the client hit (the interact packet's location) when that
    /// is on it, else at its centre.
    fn look_at_entity(
        &mut self,
        host: &mut LanHost,
        gi: usize,
        id: i32,
        location: Option<[f64; 3]>,
    ) -> Option<([f64; 3], [f64; 3])> {
        let eye = {
            let g = &host.hub.guests[gi];
            [g.position[0], g.position[1] + g.eye_height(), g.position[2]]
        };
        let object = || {
            let (lo, hi, origin) = self.object_target(id)?;
            let middle = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            let Some(l) = location else {
                // A boat's mobs sit over its middle: aim low at its near
                // side, so the ray meets the boat before them (the server
                // attacks the entity it is sent).
                let carries = u32::try_from(id).is_ok_and(|v| {
                    self.boats.iter().any(|b| b.id == v) && !self.boat_mobs(v).is_empty()
                });
                if carries {
                    let near = |k: usize| eye[k].clamp(lo[k] + 0.01, hi[k] - 0.01);
                    return Some([near(0), lo[1] + 0.05, near(2)]);
                }
                return Some(middle);
            };
            let point = [0, 1, 2].map(|k| origin[k] + l[k]);
            // A little inside the box, so the ray from the eye meets it.
            let inside = [0, 1, 2].map(|k| {
                let margin = ((hi[k] - lo[k]) / 2.0).min(0.01);
                point[k].clamp(lo[k] + margin, hi[k] - margin)
            });
            let on = (0..3).all(|k| (point[k] - inside[k]).abs() < 0.1);
            Some(if on { inside } else { middle })
        };
        let centre = match self.mobs.iter().find(|m| m.id as i32 == id) {
            Some(m) => [
                m.position.x,
                m.position.y + m.kind.height * 0.5,
                m.position.z,
            ],
            None if self.object_target(id).is_some() => object()?,
            // A dragon part or an end crystal (game/dragon.rs).
            None => {
                let b = self.tracked_special_box(id)?;
                let (lo, hi) = (b.min_position(), b.max_position());
                [
                    (lo.x + hi.x) / 2.0,
                    (lo.y + hi.y) / 2.0,
                    (lo.z + hi.z) / 2.0,
                ]
            }
        };
        let g = &mut host.hub.guests[gi];
        let d = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
        g.yaw = (-d[0].atan2(d[2])).to_degrees() as f32;
        g.pitch = (-d[1].atan2(horizontal)).to_degrees() as f32;
        Some((centre, eye))
    }

    fn guest_attack(&mut self, host: &mut LanHost, conn: u32, id: i32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if host.hub.guests[gi].spectator() {
            return;
        }
        if let Some(victim) = Self::pvp_victim(host, conn, id) {
            if let Some(Some(strike)) = self.as_guest(host, conn, |g| g.pvp_strike())
                && self.pvp_land(host, victim, &strike)
                && strike.stops_sprint()
            {
                self.as_guest(host, conn, |g| g.stop_sprint_after_extra_knockback());
            }
            return;
        }
        // A ghast's fireball or a wind charge: turned along the guest's own
        // view (ServerGamePacketListenerImpl.handleInteract, then
        // Player.attack's AIM_DEFLECT).
        if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id != 0 && p.id as i32 == id && super::deflect::redirectable(p))
        {
            self.as_guest(host, conn, |g| {
                if g.within_interaction_range(i) {
                    g.deflect(i);
                }
            });
            return;
        }
        let (old_yaw, old_pitch) = (host.hub.guests[gi].yaw, host.hub.guests[gi].pitch);
        if self.look_at_entity(host, gi, id, None).is_none() {
            return;
        }
        self.as_guest(host, conn, |g| {
            g.attack();
        });
        // The mob now answers to this guest (game/lan_mobs.rs).
        if let Ok(id) = u32::try_from(id) {
            host.mob_players.insert(id, conn);
        }
        host.hub.guests[gi].yaw = old_yaw;
        host.hub.guests[gi].pitch = old_pitch;
    }

    /// Commands.performCommand for a guest with command access.
    /// Commands.performCommand for a guest, with the message arguments it
    /// signed (none after an unsigned command packet).
    pub(super) fn guest_command(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        command: &str,
        signed: Option<Vec<(String, horizon_client::lan::hub::chat::ChatMessage)>>,
    ) {
        if self.guest_message_command(host, conn, command, signed.as_ref()) {
            return;
        }
        // ListPlayersCommand needs no permission and lists every player.
        let words: Vec<&str> = command.trim_start_matches('/').split(' ').collect();
        if words[0] == "list" && (words.len() == 1 || words[1..] == ["uuids"]) {
            let me = (self.options.dedicated.is_none() && self.player.is_some())
                .then_some((host.host_uuid, host.host_name.as_str()));
            let content = host.hub.list_reply(me, words.len() == 2);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content,
                    overlay: false,
                }),
            );
            return;
        }
        // A server mod's command, for every player whatever their level.
        if self.guest_server_mod_command(host, conn, command.trim_start_matches('/')) {
            return;
        }
        // An integrated server runs every player's commands at their own
        // permission level (`CommandSourceStack.permissions`); a LAN world
        // follows its Allow Commands, and a dedicated server leaves it to
        // the ops list (the guest's permission level).
        let integrated = host.hub.config.integrated;
        let allowed = self.options.dedicated.is_some() || self.allow_commands();
        if !integrated && (!host.settings.guest_commands || !allowed) {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_protocol::text::translatable("command.unknown.command", &[]),
                    overlay: false,
                }),
            );
            return;
        }
        // The guest's own level (Commands.performCommand with the player's
        // source): an operator's on a dedicated server, never the host's.
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        host.commands.push(GuestCommand {
            conn,
            command: command.trim_start_matches('/').to_owned(),
            permission,
            integrated,
        });
    }

    /// The guests' queued commands, run with the hub in `self.lan` so that
    /// they reach the other players too (`/give Bob`, `/tp Bob`, `/kick`).
    pub(super) fn run_guest_commands(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.commands.is_empty() {
                return;
            }
            let c = host.commands.remove(0);
            let mut host = self.lan.take().expect("checked above");
            let Some(swap) = self.enter_guest(&mut host, c.conn) else {
                self.lan = Some(host);
                continue;
            };
            self.lan = Some(host);
            self.chat_capture = Some(Vec::new());
            let level = std::mem::replace(&mut self.ui.registries.permission, c.permission);
            self.run_command(&c.command);
            self.ui.registries.permission = level;
            let lines = self.chat_capture.take().unwrap_or_default();
            let mut host = self.lan.take().expect("a command puts the hub back");
            self.leave_guest(&mut host, swap);
            host.hub.sync_all_effects();
            if !c.integrated {
                // A teleport into another level happens before its feedback is sent.
                self.run_level_teleports(&mut host);
            }
            // What the command earned is announced before its feedback.
            self.announce_guest_advancements(&mut host);
            for line in lines {
                host.hub.send_to(
                    c.conn,
                    &C::SystemChat(play::SystemChat {
                        content: horizon_client::lan::data::component_nbt(&line.component()),
                        overlay: false,
                    }),
                );
            }
            // The other operators hear of it; never the player whose command it was.
            self.send_admin_feedback(&mut host, Some(c.conn));
            self.lan = Some(host);
        }
    }

    /// The `broadcastToAdmins` lines a command queued (`source`: the guest
    /// whose it was; none for the host's or the console's): every other
    /// operator gets them, and the console logs them as the server's own
    /// system messages (MinecraftServer.sendSystemMessage).
    pub(super) fn send_admin_feedback(&mut self, host: &mut LanHost, source: Option<u32>) {
        // PlayerList.isOp for the host of a world open to LAN: the owner when
        // the world allows commands, everyone when the LAN does.
        let host_op = source.is_some()
            && !host.hub.config.dedicated
            && (self.allow_commands() || host.hub.config.guest_commands);
        for (line, ops, console) in std::mem::take(&mut self.admin_feedback) {
            if ops {
                let content = horizon_client::lan::data::component_nbt(&line.component());
                for g in host
                    .hub
                    .guests
                    .iter()
                    .filter(|g| Some(g.conn) != source && g.permission > 0)
                {
                    g.send(&C::SystemChat(play::SystemChat {
                        content: content.clone(),
                        overlay: false,
                    }));
                }
                if host_op {
                    self.system_message(line.clone());
                }
            }
            if console {
                super::dedicated::log("INFO", &format!("System chat: {}", line.plain()));
            }
        }
    }

    /// ServerGamePacketListenerImpl.handleCustomCommandSuggestions: the
    /// completions of the command typed so far (after an optional slash), for
    /// the nodes the tree marks `ask_server`.
    fn guest_suggestions(&mut self, host: &mut LanHost, conn: u32, payload: &[u8]) {
        let mut r = horizon_protocol::buf::Reader::new(payload);
        let (Ok(id), Ok(text)) = (r.varint(), r.string(32500)) else {
            return;
        };
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        let skip = usize::from(text.starts_with('/'));
        let command = &text[skip..];
        let d = vanilla::game_dispatcher();
        let level = std::mem::replace(&mut self.ui.registries.permission, permission);
        let parse = d.parse(command, &self.ui.registries);
        let ((start, end), suggestions) = d.suggestions(&parse, command.len(), &self.ui.registries);
        self.ui.registries.permission = level;
        let utf16 = |byte: usize| command[..byte.min(command.len())].encode_utf16().count() as i32;
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(id);
        w.varint(utf16(start) + skip as i32);
        w.varint(utf16(end) - utf16(start));
        w.varint(suggestions.len() as i32);
        for s in &suggestions {
            w.string(&s.text);
            w.bool(s.tooltip.is_some());
            if let Some(tip) = &s.tooltip {
                w.nbt(&literal(tip));
            }
        }
        host.hub.send_to(
            conn,
            &C::Unknown(
                horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS,
                w.data,
            ),
        );
    }

    fn guest_respawn(&mut self, host: &mut LanHost, conn: u32) {
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        // The guest's own bed, else the world spawn: never the host's bed.
        let world = self
            .level
            .world_spawn
            .map(|[x, y, z]| [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]);
        let spawn = host.hub.guests[i]
            .spawn
            .or(world)
            .or(self.spawn.map(|s| [s.x, s.y, s.z]))
            .unwrap_or([0.5, 80.0, 0.5]);
        if !host.hub.guests[i].dead {
            return;
        }
        // PlayerList.respawn: a server's player comes back in its respawn
        // point's level: the Overworld, where the world spawn and beds are,
        // or another one's anchor or forced point, which its block resolves
        // once the level has loaded around it.
        let server = self.options.dedicated.is_some();
        let own = host.hub.guests[i]
            .respawn_elsewhere
            .as_ref()
            .map(RespawnConfig::from_saved);
        let elsewhere = own.filter(|_| server);
        // In the window's world a guest has no level of its own to wait in:
        // its bed is resolved at once when its chunk is there, else it
        // comes back on the bed.
        let in_frame_bed = own.filter(|c| !server && c.dim == super::Dim::Overworld);
        let spawn = in_frame_bed.map_or(spawn, |c| {
            [
                f64::from(c.pos[0]) + 0.5,
                f64::from(c.pos[1]) + 0.6,
                f64::from(c.pos[2]) + 0.5,
            ]
        });
        let level = elsewhere.map_or(super::Dim::Overworld, |c| c.dim);
        let spawn = elsewhere.map_or(spawn, |c| {
            [
                f64::from(c.pos[0]) + 0.5,
                f64::from(c.pos[1]),
                f64::from(c.pos[2]) + 0.5,
            ]
        });
        let back = self.dim;
        let away = server && host.hub.guests[i].dimension != level.id();
        if server {
            self.server_trips.retain(|t| !t.is(conn));
        }
        if away && let Err(e) = self.enter_level(Some(host), level) {
            log::error!("could not open {}: {e}", level.id());
            return;
        }
        let g = &mut host.hub.guests[i];
        g.dead = false;
        g.health = 20.0;
        g.food = 20;
        g.saturation = 5.0;
        g.sent_health = None;
        if let Some(m) = g.motion.take() {
            let m = *m;
            g.logs_kept = Some((m.hurt_log, m.attack_log));
        }
        let mut xp = Experience {
            level: g.xp_level,
            progress: g.xp_progress,
            total: g.xp_total,
        };
        xp.respawn(self.ui.rules.bool("keep_inventory") || g.game_mode == SPECTATOR);
        (g.xp_level, g.xp_progress, g.xp_total) = (xp.level, xp.progress, xp.total);
        // A new body: nothing given to the old one carries over.
        if let Some(play) = host.play.get_mut(&conn) {
            play.potions = Default::default();
        }
        let clock_rate = self.network_clock_rate();
        let ctx = TickCtx {
            world: &mut self.world,
            host: None,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
        };
        host.hub.respawn(i, spawn, &ctx);
        if let Some(config) = elsewhere {
            self.respawn_trip(conn, config);
        }
        if let Some(c) = in_frame_bed
            && self.dim == super::Dim::Overworld
            && self.world.loaded(c.pos[0], c.pos[2])
        {
            let found = self
                .as_guest(host, conn, |g| g.resolve_server_respawn(c))
                .flatten();
            let world = self
                .level
                .world_spawn
                .map_or([0.5, 80.0, 0.5], |[x, y, z]| {
                    [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]
                });
            let at = found.map_or(world, |(at, yaw)| {
                host.hub.guests[i].yaw = yaw;
                [at.x, at.y, at.z]
            });
            host.hub.teleport(i, Some(at));
        }
        if away && let Err(e) = self.enter_level(Some(host), back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
    }

    /// Per tick, for every guest: dig, use items, pick up drops, die.
    pub(super) fn tick_guests(&mut self, host: &mut LanHost) {
        self.lan_merchant_tick(host);
        self.lan_stations_tick(host);
        self.tick_lan_mount_menus(host);
        self.announce_guest_advancements(host);
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                continue;
            };
            self.guest_sleep_tick(host, gi);
            if host.hub.guests[gi].dead || self.on_server_trip(conn) {
                continue;
            }
            let play = host.play.entry(conn).or_default();
            // Player.tick's ItemCooldowns.tick, once a tick for each player
            // (the shield an axe disabled).
            play.shield_cooldown = play.shield_cooldown.saturating_sub(1);
            let digging = play.digging;
            let using = play.using.is_some();
            let delayed = play.destroy_delay > 0 || play.use_delay > 0 || play.attack_ticks < 100;
            if digging.is_some() || using || delayed {
                self.as_guest(host, conn, |g| {
                    g.attack_ticks = g.attack_ticks.saturating_add(1);
                    if let Some((pos, face)) = digging {
                        g.forced_hit = Some(Hit {
                            pos,
                            face,
                            point: [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]) + 0.5,
                                f64::from(pos[2]) + 0.5,
                            ],
                        });
                    }
                    let _ = g.tick_mining();
                    g.forced_hit = None;
                    g.right = g.using.is_some();
                    g.tick_using();
                    if g.use_delay > 0 {
                        g.use_delay -= 1;
                    }
                });
                // ServerPlayerGameMode.handleBlockBreakAction: a creative
                // player's START breaks the block once (destroyAndAck) and
                // leaves nothing being destroyed; its client sends no STOP.
                // Kept, the dig broke whatever was put there next, every
                // destroy delay, for as long as the player broke nothing else.
                if digging.is_some()
                    && host.hub.guests[gi].creative()
                    && let Some(play) = host.play.get_mut(&conn)
                {
                    play.digging = None;
                }
            }
            // Player.tick: a different main-hand item resets the attack
            // strength, and detectEquipmentUpdates gives the guest its modifiers.
            let g = &host.hub.guests[gi];
            let held = g
                .inventory
                .get(g.selected)
                .cloned()
                .flatten()
                .map(|s| s.item);
            let play = host.play.entry(conn).or_default();
            if play.attack_item != held {
                play.attack_ticks = 0;
                play.attack_item = held;
            }
            self.guest_delayed_destroy(host, conn);
            self.guest_server_tick(host, conn);
            // Player.tick's statistics and ServerPlayer.tick's triggers, then
            // PlayerAdvancements.flushDirty.
            let sleeping = host.hub.guests[gi].sleeping;
            self.as_guest(host, conn, |g| {
                g.tick_fishing();
                g.tick_maps();
                g.count_tick_stats(sleeping);
                g.tick_advancements();
                g.update_player_attributes();
            });
            self.lan_map_updates(host, conn);
            host.hub.flush_progress(conn);
            self.guest_dig_feedback(host, conn);
            self.guest_pickups(host, gi);
            self.guest_orbs(host, gi);
            if host.hub.guests[gi].health <= 0.0 && !host.hub.guests[gi].dead {
                self.guest_forget_digging(host, conn);
                // FishingHook.shouldStopFishing: its owner is dead.
                if let Some(play) = host.play.get_mut(&conn) {
                    play.hook = None;
                }
            }
            let g = &mut host.hub.guests[gi];
            if g.health <= 0.0 && !g.dead {
                g.dead = true;
                g.last_death = Some(g.position.map(|v| v.floor() as i32));
                let (id, at) = (g.entity_id, g.position);
                let reward = Experience {
                    level: g.xp_level,
                    progress: g.xp_progress,
                    total: g.xp_total,
                }
                .death_reward(self.ui.rules.bool("keep_inventory") || g.game_mode == SPECTATOR);
                let (name, uuid) = (g.name.clone(), g.uuid);
                // ServerPlayer.die: dropAllDeathLoot unless a spectator, from the eyes less 0.3.
                let drops = (g.game_mode != SPECTATOR)
                    .then(|| [at[0], at[1] + g.eye_height() - 0.3, at[2]]);
                let cause = host.play.get_mut(&conn).and_then(|p| p.death_cause.take());
                let message = match cause {
                    // ServerPlayer.die: the death message, to everyone unless
                    // the player's team hides it.
                    Some(key) if self.ui.rules.bool("show_death_messages") => {
                        let text = horizon_client::text::Text::translate(
                            &key,
                            &[self.player_text(&name, uuid)],
                        );
                        let message = horizon_client::lan::data::component_nbt(&text.component());
                        let team = self.score.board.players_team(&name);
                        if horizon_client::scoreboard::own_death_message_shown(team) {
                            host.hub.system_message(message.clone());
                            if self.options.dedicated.is_some() {
                                super::dedicated::log(
                                    "INFO",
                                    &format!("System chat: {}", text.plain()),
                                );
                            } else if self.player.is_some() {
                                self.system_message(text);
                            }
                        }
                        message
                    }
                    Some(_) => literal(""),
                    None => literal(&format!("{name} died")),
                };
                host.hub.send_to(
                    conn,
                    &C::PlayerCombatKill(play::PlayerCombatKill { id, message }),
                );
                // LivingEntity.die: the death animation, the dying player included.
                host.hub.entity_event(id, 3, at);
                if let Some(eyes) = drops {
                    self.as_guest(host, conn, |game| game.drop_inventory(eyes));
                }
                self.spawn_orbs(at, reward);
            }
        }
    }

    /// `Player.tick`'s sleep counter, and `LivingEntity.tick`'s wake-up of a
    /// sleeper who died or whose bed is gone.
    fn guest_sleep_tick(&mut self, host: &mut LanHost, gi: usize) {
        let g = &mut host.hub.guests[gi];
        g.sleep_timer = super::tick_sleep_timer(g.sleep_timer, g.bed.is_some());
        if let Some(head) = g.bed {
            let block = self.world.block(head[0], head[1], head[2]);
            let bed = self.res.facts().get(block).name.ends_with("_bed");
            if g.dead || g.health <= 0.0 || !bed {
                host.hub.wake(gi);
            }
        }
    }

    /// `SleepStatus.areEnoughSleeping` and `areEnoughDeepSleeping` over the
    /// host's player and the guests: of the players not in spectator mode,
    /// players_sleeping_percentage of them (at least one) are in bed, and as
    /// many have slept 100 ticks.
    pub(super) fn enough_slept(&self) -> bool {
        let (mut active, mut sleeping, mut deep) = (0i64, 0i64, 0i64);
        let mut count = |in_bed: bool, timer: u32| {
            active += 1;
            if in_bed {
                sleeping += 1;
                if timer >= 100 {
                    deep += 1;
                }
            }
        };
        if self.player.is_some() && !self.spectator() {
            count(self.sleeping.is_some(), self.sleep_timer);
        }
        if let Some(host) = self.lan.as_ref() {
            for g in host
                .hub
                .guests
                .iter()
                .filter(|g| g.placed && !g.spectator())
            {
                count(g.bed.is_some(), g.sleep_timer);
            }
        }
        let percent = i64::from(self.ui.rules.int("players_sleeping_percentage").max(0));
        let needed = ((active * percent + 99) / 100).max(1);
        sleeping >= needed && deep >= needed
    }

    /// `ServerLevel.wakeUpAllPlayers` for the guests.
    pub(super) fn wake_guests(&mut self) {
        if let Some(host) = self.lan.as_mut() {
            for i in 0..host.hub.guests.len() {
                host.hub.wake(i);
            }
        }
    }

    /// `PlayerList.broadcastSystemMessage` of the advancements guests made
    /// (their own actions, commands and ticks).
    pub(super) fn announce_guest_advancements(&mut self, host: &mut LanHost) {
        for text in std::mem::take(&mut self.adv.announced) {
            host.hub
                .system_message(horizon_client::lan::data::component_nbt(&text.component()));
            if self.options.dedicated.is_none() {
                self.system_message(text);
            }
        }
    }

    /// The guest's body on the server each tick: ServerPlayer.tick's damage
    /// clock and Entity.baseTick's fluid phase (burning), then for each move
    /// the guest made ServerGamePacketListenerImpl.handleMovePlayer's jump
    /// (`ServerPlayer.jumpFromGround`'s exhaustion),
    /// ServerPlayer.doCheckFallDamage (the fall distance grows while they go
    /// down and hurts them where they land, as the host's own landing does;
    /// water resets it, lava halves it) and checkMovementStatistics (the
    /// exhaustion of sprinting and swimming), then LivingEntity.baseTick's
    /// hazards (drowning, lava, fire blocks, suffocation, the void), and
    /// regeneration and the food tick (ServerPlayer.doTick). Their hurt
    /// sounds are heard at the guest.
    fn guest_server_tick(&mut self, host: &mut LanHost, conn: u32) {
        self.guest_vehicle_tick(host, conn);
        // The last move's fall stands in for the server's delta movement.
        let fell = host
            .hub
            .guest_mut(conn)
            .and_then(|g| g.moves.last().map(|m| m.0[1]))
            .unwrap_or(0.0);
        self.guest_glide_tick(host, conn, fell);
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        let moves = std::mem::take(&mut g.moves);
        let from_ground = if moves.is_empty() {
            g.on_ground
        } else {
            g.moves_from_ground
        };
        if g.spectator() {
            // Spectators' effects still run (LivingEntity.baseTick).
            if !g.effects.list.is_empty() {
                self.as_guest(host, conn, |game| game.tick_mob_effects());
            }
            return;
        }
        let in_wall = !g.sleeping && self.in_wall(g.position, g.eye_height());
        let at = g.position;
        let sneaking = g.sneaking;
        let events = self.as_guest(host, conn, |game| {
            let (rules, motion) = (game.world.health_rules, game.motion);
            let health = &motion.health;
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None);
            };
            let mut w = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
            };
            w.player.health.advance_clock();
            let _ = w.fluid_base_tick();
            w.player.ground.on_ground = from_ground;
            let travelled: Vec<[f64; 3]> = moves.iter().map(|m| m.0).collect();
            // Entity.move's movementThisTick: each move, from where the
            // tick's first one started.
            let end = w.player.position;
            let start = travelled
                .iter()
                .fold(end, |p, d| p.subtract(Vec3::new(d[0], d[1], d[2])));
            w.player.history.clear();
            let mut from = start;
            for d in &travelled {
                let delta = Vec3::new(d[0], d[1], d[2]);
                let to = from.add_vector(delta);
                w.player
                    .history
                    .add(horizon_world::entity_move::MovementRecord {
                        from,
                        to,
                        original: Some(delta),
                    });
                from = to;
            }
            for (moved, on_ground) in moves {
                let movement = Vec3::new(moved[0], moved[1], moved[2]);
                if w.player.ground.on_ground && !on_ground && moved[1] > 0.0 {
                    let sprinting = w.player.sprinting;
                    w.player
                        .cause_food_exhaustion(if sprinting { 0.2 } else { 0.05 });
                }
                let landed = (|| -> horizon_client::Result<()> {
                    w.set_on_ground_with_movement(on_ground, false, movement)?;
                    let pos = w.legacy_on_position()?;
                    let state = w.block_state(pos)?;
                    w.check_fall_damage(moved[1], on_ground, state, pos)?;
                    w.check_movement_statistics(movement)
                })();
                if landed.is_err() {
                    break;
                }
            }
            // A ServerPlayer travels on the server too
            // (MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER), so every tick
            // on a ladder or vine LivingEntity.handleOnClimbable resets its fall
            // distance: climbing down doesn't hurt where it lands (#1255).
            if w.climbable().unwrap_or(false) {
                w.player.fall_distance = 0.0;
            }
            // Entity.applyEffectsFromBlocks along those moves: magma, cactus,
            // lit campfires and grown sweet berry bushes hurt (#1305).
            w.player.shift_key_down = sneaking;
            let _ = w.server_block_damage(start);
            let _ = w.tick_environment_damage();
            let _ = w.apply_fire_contact();
            let _ = w.server_hazards_tick(in_wall);
            let mut events = std::mem::take(&mut w.player.events);
            // ServerGamePacketListenerImpl.handleMovePlayer:
            // Player.checkMovementStatistics for each move.
            for d in travelled {
                game.movement_stats(Vec3::new(d[0], d[1], d[2]));
            }
            // LivingEntity.baseTick: outside the world border, then tickEffects.
            game.border_damage();
            game.tick_mob_effects();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None);
            };
            let _ = player.peaceful_regeneration(rules);
            let _ = player.food_tick(health, rules);
            events.append(&mut player.events);
            let _ = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
            }
            .server_travel();
            // A hurt since the last tick marked the velocity (Entity.markHurt):
            // ServerEntity.sendChanges sends it as it is after this travel.
            let marked = std::mem::take(&mut player.health.sync_velocity);
            (events, marked.then_some(player.velocity))
        });
        let Some((events, marked)) = events else {
            return;
        };
        for event in events {
            match event {
                MotionEvent::HurtSound(sound) => self.player_hurt_sound(sound, at),
                MotionEvent::Fall(event) => self.landing_block_updates(&event),
                _ => {}
            }
        }
        if let (Some(v), Some(g)) = (marked, host.hub.guest(conn)) {
            g.send(&C::SetEntityMotion(play::SetEntityMotion {
                id: g.entity_id,
                velocity: [v.x, v.y, v.z],
            }));
        }
    }

    /// ServerGamePacketListenerImpl.handlePlayerCommand START_FALL_FLYING:
    /// `Player.tryToStartFallFlying`, or else `stopFallFlying` (a second
    /// start while gliding ends the glide, as in vanilla).
    fn lan_start_gliding(&mut self, host: &mut LanHost, conn: u32) {
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        let start = !g.fall_flying && self.guest_can_glide(host, conn) == Some(true);
        let in_liquid = self
            .as_guest(host, conn, |game| {
                game.player
                    .as_ref()
                    .is_some_and(|p| p.fluids.was_touching_water || p.is_in_lava())
            })
            .unwrap_or(true);
        if let Some(g) = host.hub.guest_mut(conn) {
            g.fall_flying = start && !in_liquid;
        }
    }

    /// `Player.canGlide`: not flying, airborne, not riding, no levitation, and
    /// a glider that its next point of wear won't break.
    fn guest_can_glide(&mut self, host: &mut LanHost, conn: u32) -> Option<bool> {
        let g = host.hub.guest_mut(conn)?;
        if g.flying || g.on_ground || g.vehicle.is_some() {
            return Some(false);
        }
        self.as_guest(host, conn, |game| {
            game.glider_worn() && game.potions.effects.get("levitation").is_none()
        })
    }

    /// `LivingEntity.updateFallFlying` on the server, once a tick for a
    /// gliding guest: the glide ends once `canGlide` fails; otherwise every
    /// 20th tick wears the glider by one, and every 10th is an
    /// `elytra_glide` game event. Entity.checkFallDistanceAccumulation runs
    /// first: a glide that isn't diving keeps the fall distance at 1.
    fn guest_glide_tick(&mut self, host: &mut LanHost, conn: u32, fell: f64) {
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        if !g.fall_flying {
            if let Some(m) = g.motion.as_mut() {
                m.fall_fly_ticks = 0;
            }
            return;
        }
        let creative = g.creative();
        if let Some(m) = g.motion.as_mut()
            && fell > -0.5
            && m.fall_distance > 1.0
        {
            m.fall_distance = 1.0;
        }
        if self.guest_can_glide(host, conn) != Some(true) {
            if let Some(g) = host.hub.guest_mut(conn) {
                g.fall_flying = false;
            }
            return;
        }
        self.as_guest(host, conn, |game| {
            let Some(p) = game.player.as_mut() else {
                return;
            };
            p.fall_fly_ticks += 1;
            let next = p.fall_fly_ticks + 1;
            let at = [p.position.x, p.position.y, p.position.z];
            if next % 10 != 0 {
                return;
            }
            if (next / 10) % 2 == 0
                && !creative
                && let Some(elytra) = game.armor[1].as_mut()
            {
                elytra.damage += 1;
            }
            let ctx = game.player_context(None);
            game.post_game_event("minecraft:elytra_glide", at, ctx);
        });
    }

    /// `Player.isInWall`: a suffocating block in the 0.48-wide box at the
    /// eyes. Suffocating is a full collision box that is also a full render
    /// cube (`BlockBehaviour.isSuffocating`'s default; glass and the like
    /// opt out), as the mobs' check has it.
    fn in_wall(&self, at: [f64; 3], eye_height: f64) -> bool {
        let (y, half) = ((at[1] + eye_height).floor() as i32, 0.24);
        let range = |v: f64| (v - half).floor() as i32..=(v + half).floor() as i32;
        range(at[0]).any(|x| {
            range(at[2]).any(|z| {
                let f = self.res.facts().get(self.world.block(x, y, z));
                f.full_collision && f.solid_render
            })
        })
    }

    /// ServerPlayerGameMode.handleBlockBreakAction, STOP_DESTROY_BLOCK: the client finished
    /// digging `pos` (and predicted the break). The server breaks it if its own count of the
    /// digging since START is at least 70 % done; otherwise it keeps digging alone and breaks
    /// it when done (`guest_delayed_destroy`), so the client's broken block is never left
    /// standing on the server.
    fn guest_stop_digging(&mut self, host: &mut LanHost, conn: u32, pos: [i32; 3]) {
        let ticks = host.hub.ticks;
        let play = host.play.entry(conn).or_default();
        let dug = play.digging.take().is_some_and(|(p, _)| p == pos);
        play.mining = None;
        let start = play.dig_start;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !dug {
            return;
        }
        if self.res.facts().is_air(state) {
            // Already broken (the guest's own tick dug it out): only the crack goes.
            self.guest_crack_gone(host, conn, pos);
            return;
        }
        let progress = self
            .as_guest(host, conn, |g| g.destroy_progress(pos))
            .map_or(0.0, |p| p * (ticks - start + 1) as f32);
        if progress >= 0.7 {
            self.guest_destroy(host, conn, pos);
        } else {
            let play = host.play.entry(conn).or_default();
            if play.delayed.is_none() {
                play.delayed = Some((pos, start));
            }
        }
    }

    /// ServerPlayerGameMode.tick's delayed destroy: the block breaks once its progress since
    /// the digging started is complete, unless something else removed it first.
    fn guest_delayed_destroy(&mut self, host: &mut LanHost, conn: u32) {
        let Some((pos, start)) = host.play.get(&conn).and_then(|p| p.delayed) else {
            return;
        };
        let ticks = host.hub.ticks;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let progress = if self.res.facts().is_air(state) {
            None
        } else {
            self.as_guest(host, conn, |g| g.destroy_progress(pos))
                .map(|p| p * (ticks - start + 1) as f32)
        };
        match progress {
            Some(p) if p < 1.0 => {}
            done => {
                if let Some(play) = host.play.get_mut(&conn) {
                    play.delayed = None;
                }
                if done.is_some() {
                    self.guest_destroy(host, conn, pos);
                }
            }
        }
    }

    /// ServerPlayerGameMode.destroyBlock for a guest, its crack cleared for the others
    /// (`destroyBlockProgress(id, pos, -1)`).
    fn guest_destroy(&mut self, host: &mut LanHost, conn: u32, pos: [i32; 3]) {
        self.as_guest(host, conn, |g| {
            let _ = g.break_block(pos);
        });
        self.guest_crack_gone(host, conn, pos);
    }

    /// The crack animation other players see.
    fn guest_dig_feedback(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let play = host.play.entry(conn).or_default();
        let Some((pos, _)) = play.digging else {
            return;
        };
        // ServerPlayerGameMode.tick: a block gone while it is dug (the guest's own tick broke
        // it, or something else did) clears the crack.
        if play.last_progress >= 0
            && self
                .res
                .facts()
                .is_air(self.world.block(pos[0], pos[1], pos[2]))
        {
            play.last_progress = -1;
            self.guest_crack_gone(host, conn, pos);
            return;
        }
        let progress = play
            .mining
            .as_ref()
            .map_or(-1, |m| (m.progress.clamp(0.0, 1.0) * 10.0) as i32 - 1);
        let swinging = play.swing.swinging();
        if progress != play.last_progress && progress >= 0 {
            play.last_progress = progress;
            // The host player sees it too (on the server thread its window is a guest).
            self.destroy_block_progress(host.hub.guests[gi].entity_id, pos, progress);
            host.hub.broadcast_near(
                [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                64.0,
                &C::BlockDestruction(play::BlockDestruction {
                    id: host.hub.guests[gi].entity_id,
                    pos,
                    progress: progress as u8,
                }),
                Some(conn),
            );
        }
        let _ = swinging;
    }

    /// `level.destroyBlockProgress(id, pos, -1)` for a guest: its crack on `pos` is gone, for the
    /// host and the other guests.
    /// A player who dies digs and uses nothing any more: the block it was
    /// breaking, or had left to break (`hasDelayedDestroy`), stays, and its
    /// crack goes. Respawning gives it a new ServerPlayerGameMode (#1315).
    fn guest_forget_digging(&mut self, host: &mut LanHost, conn: u32) {
        let Some(play) = host.play.get_mut(&conn) else {
            return;
        };
        let cracked = play.digging.take().filter(|_| play.last_progress >= 0);
        play.mining = None;
        play.delayed = None;
        play.destroy_delay = 0;
        play.using = None;
        play.last_progress = -1;
        if let Some((pos, _)) = cracked {
            self.guest_crack_gone(host, conn, pos);
        }
    }
    fn guest_crack_gone(&mut self, host: &mut LanHost, conn: u32, pos: [i32; 3]) {
        let Some(id) = host.hub.guest(conn).map(|g| g.entity_id) else {
            return;
        };
        self.destroy_block_progress(id, pos, -1);
        host.hub.broadcast_near(
            [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
            64.0,
            &C::BlockDestruction(play::BlockDestruction {
                id,
                pos,
                progress: 255,
            }),
            Some(conn),
        );
    }

    /// Dropped items touching a guest go into their inventory.
    fn guest_pickups(&mut self, host: &mut LanHost, gi: usize) {
        let (conn, mode, position, id) = {
            let g = &host.hub.guests[gi];
            (g.conn, g.game_mode, g.position, g.entity_id)
        };
        if mode == SPECTATOR || self.dropped.is_empty() {
            return;
        }
        let near: Vec<usize> = self
            .dropped
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.pickup_delay == 0
                    && (d.position.x - position[0]).abs() < 1.5
                    && (d.position.z - position[2]).abs() < 1.5
                    && d.position.y > position[1] - 0.6
                    && d.position.y < position[1] + 2.4
            })
            .map(|(i, _)| i)
            .collect();
        let mut taken = Vec::new();
        for i in near.into_iter().rev() {
            let (stack, item_id) = (self.dropped[i].stack.clone(), self.dropped[i].id);
            let before = stack.count;
            let rest = self.as_guest(host, conn, |g| g.give(stack)).flatten();
            match rest {
                None => {
                    let item = self.dropped.remove(i);
                    self.guest_pickup(host, id, item);
                    taken.push((item_id, before));
                }
                Some(rest) if rest.count < before => {
                    self.guest_pickup(host, id, self.dropped[i].clone());
                    self.dropped[i].stack = rest;
                    taken.push((item_id, before - self.dropped[i].stack.count));
                }
                Some(_) => {}
            }
        }
        for (item, count) in taken {
            host.hub.take_item(item as i32, id, count as i32, position);
            host.hub
                .sound("minecraft:entity.item.pickup", position, 0.2, 1.4, 7, None);
        }
    }

    /// `Player.tick`'s takeXpDelay and `Player.aiStep`'s touch of the
    /// experience orbs in the guest's box grown by 1, 0.5, 1: one of them at
    /// random, at most every 2 ticks (`ExperienceOrb.playerTouch`), repairs
    /// with Mending and gives the rest as points. Everyone near hears it.
    fn guest_orbs(&mut self, host: &mut LanHost, gi: usize) {
        let g = &host.hub.guests[gi];
        // Player's standing and crouching boxes: 1.8 and 1.5 high.
        let height = if g.sneaking { 1.5 } else { 1.8 };
        let (conn, id, at) = (g.conn, g.entity_id, g.position);
        let unable = g.spectator() || g.dead || g.health <= 0.0;
        let play = host.play.entry(conn).or_default();
        if play.xp_delay > 0 {
            play.xp_delay -= 1;
        }
        if unable || play.xp_delay > 0 {
            return;
        }
        let (min, max) = (
            [at[0] - 1.3, at[1] - 0.5, at[2] - 1.3],
            [at[0] + 1.3, at[1] + height + 0.5, at[2] + 1.3],
        );
        let touching: Vec<usize> = self
            .orbs
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2]
            })
            .map(|(i, _)| i)
            .collect();
        if touching.is_empty() {
            return;
        }
        // Util.getRandom(orbs, random).
        let pick = self
            .world
            .random
            .next_int_bound(touching.len() as i32)
            .unwrap_or(0) as usize;
        let orb = self.orbs.remove(touching[pick]);
        play.xp_delay = 2;
        host.hub.take_item(orb.id as i32, id, 1, at);
        // ClientPacketListener.handleTakeItemEntity's sound, at the orb.
        let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
        let p = orb.position;
        self.audio.play(
            "minecraft:entity.experience_orb.pickup",
            Some([p.x, p.y, p.z]),
            0.1,
            pitch,
        );
        let value = orb.value;
        if let Some(target) = super::pickups::guest_target(host, id as u32) {
            use super::pickups::{Collector, Taken};
            self.add_pickup_toward(
                Taken::Orb(Box::new(orb)),
                Collector::Guest(id as u32),
                target,
            );
        }
        self.as_guest(host, conn, |game| {
            let rest = game.mending(value);
            if rest > 0 {
                game.give_orb_points(rest);
            }
        });
    }

    /// The host's own `ItemPickupParticle` toward the guest's model.
    fn guest_pickup(
        &mut self,
        host: &LanHost,
        id: i32,
        item: horizon_client::entities::ItemEntity,
    ) {
        use super::pickups::{Collector, Taken, guest_target};
        if let Some(target) = guest_target(host, id as u32) {
            self.add_pickup_toward(
                Taken::Item(Box::new(item)),
                Collector::Guest(id as u32),
                target,
            );
        }
    }

    /// `level.playSound(player, ...)` of a guest's use it predicted (`remote::hand_operated`
    /// blocks only, as the guest decides): the others hear it now, the guest already did.
    fn guest_use_sounds(&mut self, host: &mut LanHost, conn: u32, logged: usize, heard: usize) {
        let changed: Vec<(i32, i32, i32)> = self
            .world
            .block_log
            .as_ref()
            .map_or(&[][..], |log| log.get(logged..).unwrap_or(&[]))
            .to_vec();
        let facts = self.res.facts();
        // BlockItem.place: `playSound(player, ...)`, for the others only. The
        // placer's client plays it as its block appears.
        let placed: Vec<([f64; 3], String)> = changed
            .iter()
            .map(|&(x, y, z)| {
                let at = [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5];
                let name = &facts.get(self.world.block(x, y, z)).sounds.place;
                (at, name.trim_start_matches("minecraft:").to_owned())
            })
            .collect();
        if let Some(capture) = self.audio.capture.as_mut() {
            let mut i = heard.min(capture.len());
            while i < capture.len() {
                let (name, at, ..) = &capture[i];
                let place = placed
                    .iter()
                    .any(|(p, n)| p == at && name.trim_start_matches("minecraft:") == n.as_str());
                if place {
                    let (name, at, volume, pitch) = capture.remove(i);
                    let source = horizon_client::settings::sound_source(&name) as i32;
                    host.hub.sound(&name, at, volume, pitch, source, Some(conn));
                } else {
                    i += 1;
                }
            }
        }
        if changed.is_empty()
            || !changed.iter().all(|&(x, y, z)| {
                super::remote::hand_operated(&facts.get(self.world.block(x, y, z)).name)
            })
        {
            return;
        }
        let Some(capture) = self.audio.capture.as_mut() else {
            return;
        };
        let sounds: Vec<_> = capture.drain(heard.min(capture.len())..).collect();
        for (name, at, volume, pitch) in sounds {
            let source = horizon_client::settings::sound_source(&name) as i32;
            host.hub.sound(&name, at, volume, pitch, source, Some(conn));
        }
    }

    /// Positional sounds and level events of the tick, for the guests.
    pub(super) fn lan_broadcast_effects(&mut self, host: &mut LanHost) {
        let sounds = self.audio.capture.take().unwrap_or_default();
        self.audio.capture = Some(Vec::new());
        let blocks: Vec<[f64; 3]> = std::mem::take(&mut host.broken);
        for (name, at, volume, pitch) in sounds {
            // Block breaks are heard through the level event.
            if blocks
                .iter()
                .any(|b| (0..3).all(|k| (b[k] + 0.5 - at[k]).abs() < 1e-6))
            {
                continue;
            }
            let source = horizon_client::settings::sound_source(&name) as i32;
            host.hub.sound(&name, at, volume, pitch, source, None);
        }
    }

    /// The guests' items in use, for the players around (`LivingEntity.startUsingItem`
    /// and `stopUsingItem` set DATA_LIVING_ENTITY_FLAGS).
    pub(super) fn lan_living_flags(&mut self, host: &mut LanHost) {
        for g in &mut host.hub.guests {
            let using = host.play.get(&g.conn).and_then(GuestPlay::using);
            g.living_flags = super::lan::living_flags(using);
        }
    }

    /// A use the server ran swung the guest's hand: their own client is told
    /// too (`swing(hand, true)` for a SwingSource.SERVER result). Horizon's
    /// client does not run uses itself, so it does not predict the swing; a
    /// client that did (a block placed) is still in the first half of its
    /// swing and ignores it (`SwingState.startIfAble`).
    fn lan_swing_back(&mut self, host: &mut LanHost, conn: u32, swung: Option<bool>) {
        if swung != Some(true) {
            return;
        }
        let (Some(g), Some(play)) = (host.hub.guest(conn), host.play.get(&conn)) else {
            return;
        };
        if let Some(c) = play.swing.current {
            host.hub
                .send_to(conn, &swing_packet(g.entity_id, c.hand, c.animation));
        }
    }

    /// The tick's arm swings for the players around (the guests' to everyone
    /// else, the host's to the guests), and `LivingEntity.updateSwingTime`
    /// for the guests' bodies here.
    pub(super) fn lan_swings(&mut self, host: &mut LanHost) {
        let conns: Vec<u32> = host.play.keys().copied().collect();
        for conn in conns {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            let (id, at, here) = (g.entity_id, g.position, host.hub.here(g));
            let play = host.play.get_mut(&conn).expect("play");
            let swung = std::mem::take(&mut play.swings) > 0;
            if let (true, true, Some(c)) = (swung, here, play.swing.current) {
                let packet = swing_packet(id, c.hand, c.animation);
                host.hub.broadcast_near(at, 128.0, &packet, Some(conn));
                if let Some(m) = host.models.get_mut(&conn) {
                    m.swing();
                }
            }
            play.swing.tick();
        }
        self.lan_host_crack(host);
        if self.swings_started != host.host_swings {
            host.host_swings = self.swings_started;
            if let (Some(p), Some(c)) = (self.player.as_ref(), self.swing.current) {
                let at = [p.position.x, p.position.y, p.position.z];
                let packet = swing_packet(host.host_entity, c.hand, c.animation);
                host.hub.broadcast_near(at, 128.0, &packet, None);
            }
        }
    }
}

impl Game<'_> {
    /// The host player's crack, for the guests (`ServerLevel.destroyBlockProgress` from the
    /// host's mining): each new stage, and -1 once it stops or breaks the block.
    fn lan_host_crack(&mut self, host: &mut LanHost) {
        let now = self
            .mining
            .as_ref()
            .and_then(|m| (m.progress > 0.0).then(|| (m.pos, ((m.progress * 10.0) as i32).min(9))));
        if now == host.host_crack {
            return;
        }
        let packet = |pos: [i32; 3], progress: u8| {
            C::BlockDestruction(play::BlockDestruction {
                id: host.host_entity,
                pos,
                progress,
            })
        };
        let near = |pos: [i32; 3]| [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        if let Some((pos, _)) = host.host_crack
            && now.is_none_or(|(p, _)| p != pos)
        {
            host.hub
                .broadcast_near(near(pos), 64.0, &packet(pos, 255), None);
        }
        if let Some((pos, stage)) = now {
            host.hub
                .broadcast_near(near(pos), 64.0, &packet(pos, stage as u8), None);
        }
        host.host_crack = now;
    }
}

/// ClientboundSwingAnimationPacket: the hand and the animation (its type and
/// base duration; each client applies the swinger's haste and fatigue).
fn swing_packet(id: i32, hand: Hand, animation: SwingAnimation) -> C {
    C::SwingAnimation(play::SwingAnimation {
        id,
        hand: i32::from(hand == Hand::Off),
        kind: i32::from(animation.kind),
        duration: animation.duration,
    })
}
