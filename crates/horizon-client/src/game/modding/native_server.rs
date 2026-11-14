//! Server commands and players for native mods (docs/modding/src/how-to/native-mods.md,
//! "Server commands and players"): the commands mods add to the
//! server's command tree (`native_register_commands`), a player's command
//! reaching them (`native_run_mod_command`), and the helpers a server mod
//! answers with: the players online, a line to one of them or to all, the
//! command's feedback and the world's folder; and the acting
//! player's plain items: count, take, give. They act on the server's own
//! players (the hub's: a dedicated server's, a LAN host's, single player's
//! server thread's), never on a client's.
#[path = "native_server/argument_kinds.rs"]
mod argument_kinds;
#[cfg(test)]
#[path = "native_server/selector_tests.rs"]
mod selector_tests;
#[cfg(test)]
#[path = "native_server/wasm_selector_tests.rs"]
mod wasm_selector_tests;

use super::api_values::{TryIntoApi, TryIntoEngine};
use super::{Dim, Game, commands::execute, lan::LanHost, respawn_anchor::RespawnConfig};
use crate::game::level_control_hooks::LevelControl;
pub(super) use argument_kinds::{CommandArguments, command_arguments};
use argument_kinds::{
    UNCENTERED, argument_type, custom_value, dimension_value, nbt_value, parsed_type, parser,
    positional_value, resource_parser,
};
use mod_api::__private::game::*;
// Their old home, for mixin code that names them here.
use horizon_client::{
    command::{Arg, ArgType, Context, Dispatcher},
    items::ItemStack,
    lan::{abilities::PlayerAbilities, data::component_nbt, guest::Guest},
    motion::PlayerMotion,
    text::Text,
};
use horizon_core::position::BlockPos;
use horizon_protocol::packets::play::{self, Clientbound as C};
use mod_api::LevelRef;
use mod_api::{
    ActionOutcome, ActionRefused, FakeEffect, GameMode, GlobalPos, Identifier, PlayerAction,
    PlayerActionSetup, PlayerFood, PlayerRef, PlayerRespawnConfig, ScopedActionResult,
};
use serde_json::{Map, Value, json};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// A sidebar a server mod shows one player (`native_sidebar`): its title and
/// up to 15 lines, top first, each a text on the left and, where the score's
/// number would be, an optional text on the right.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeSidebar {
    pub(crate) title: Text,
    pub(crate) lines: Vec<(Text, Option<Text>)>,
}

/// The objective a mod's sidebar is sent as: the player's client only; the
/// server's scoreboard never holds it.
const SIDEBAR_OBJECTIVE: &str = "horizon:native_sidebar";

/// The report's name of an operator level (`commands.json`'s
/// `permissions`).
const LEVELS: [&str; 5] = ["", "moderators", "gamemasters", "admins", "owners"];

/// One parsed word of a usage: its key in the tree and its node, without
/// children.
fn usage_nodes(
    usage: &str,
    custom: &CommandArguments,
) -> Result<Vec<(String, Value, bool)>, String> {
    let mut out = Vec::new();
    let mut optional_seen = false;
    let mut greedy_seen = false;
    for word in usage.split_whitespace() {
        if greedy_seen {
            return Err(format!("`{usage}`: a text argument must be last"));
        }
        let (inner, optional) = if let Some(w) = word.strip_prefix('<') {
            (w.strip_suffix('>'), false)
        } else if let Some(w) = word.strip_prefix('[') {
            (w.strip_suffix(']'), true)
        } else {
            if optional_seen {
                return Err(format!("`{usage}`: `{word}` follows an optional argument"));
            }
            let literal = !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c));
            if !literal {
                return Err(format!("`{usage}`: `{word}` is not a command word"));
            }
            out.push((word.to_owned(), json!({"type": "literal"}), false));
            continue;
        };
        let Some((name, kind)) = inner.and_then(|w| w.split_once(':')) else {
            return Err(format!("`{usage}`: `{word}` is not `<name:kind>`"));
        };
        if optional_seen && !optional {
            return Err(format!("`{usage}`: `{word}` follows an optional argument"));
        }
        let resource = kind
            .strip_prefix("resource:")
            .map(|registry| resource_parser(registry).map_err(|e| format!("`{usage}`: {e}")))
            .transpose()?;
        let builtin =
            resource.or_else(|| parser(kind).map(|(parser, properties, _)| (parser, properties)));
        let (parser, properties, greedy, custom) = match builtin {
            Some((parser, properties)) => (parser, properties, kind == "text", None),
            None => {
                let Some(argument) = custom.get(kind) else {
                    return Err(format!("`{usage}`: unknown kind `{kind}`"));
                };
                let (properties, greedy) = match std::panic::catch_unwind(
                    std::panic::AssertUnwindSafe(|| argument.kind()),
                )
                .map_err(|_| format!("`{usage}`: argument type `{kind}` panicked"))?
                {
                    mod_api::CommandArgumentKind::Word => (json!({"type": "word"}), false),
                    mod_api::CommandArgumentKind::Greedy => (json!({"type": "greedy"}), true),
                    _ => {
                        return Err(format!(
                            "`{usage}`: argument type `{kind}` uses a kind this build cannot read"
                        ));
                    }
                };
                ("brigadier:string", properties, greedy, Some(kind))
            }
        };
        optional_seen |= optional;
        greedy_seen |= greedy;
        let mut node = json!({"type": "argument", "parser": parser});
        if !properties.is_null() {
            node["properties"] = properties;
        }
        if kind == "vec3_uncentered" {
            node[UNCENTERED] = true.into();
        }
        if let Some(id) = custom {
            // A client that does not know the type parses a word (or the rest of the line).
            // Fabric sends the registered type's id, which an unmodded client cannot read and
            // whose node it drops; a Brigadier string keeps the tree valid and asks the server
            // for the completions.
            node["custom_argument"] = id.into();
            node["suggestions"] = "minecraft:ask_server".into();
        }
        out.push((name.to_owned(), node, optional));
    }
    if out.first().is_none_or(|n| n.1["type"] != "literal") {
        return Err(format!("`{usage}`: a command starts with its name"));
    }
    Ok(out)
}

/// Native mods' commands as a root in the commands report's format
/// (`lan::commands`, which sends each player what their level allows), and
/// each command's traversal level: the lowest of its usages. A node is runnable where
/// only optional arguments follow it, at the lowest level of those executable usages.
/// A command vanilla has (`vanilla`
/// answers) or a usage that doesn't parse is left out, with why.
pub(super) fn command_tree(
    commands: &[mod_api::Command],
    vanilla: impl Fn(&str) -> bool,
    custom: &CommandArguments,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    let mut root: Map<String, Value> = Map::new();
    let mut levels: HashMap<String, u8> = HashMap::new();
    let mut problems = Vec::new();
    for command in commands {
        let nodes = match usage_nodes(&command.usage, custom) {
            Ok(n) => n,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        if vanilla(&nodes[0].0) {
            problems.push(format!("`{}`: vanilla has /{}", command.usage, nodes[0].0));
            continue;
        }
        // A rejected usage must not open an earlier executable prefix or lower its level.
        let mut children = &root;
        let mut conflict = None;
        for (name, node, _) in &nodes {
            let Some(entry) = children.get(name) else {
                break;
            };
            // `word` and `text` share a parser; their properties tell them apart, and a custom
            // kind shares the word (or greedy) properties with them.
            if entry["type"] != node["type"]
                || entry["parser"] != node["parser"]
                || entry["properties"] != node["properties"]
                || entry["custom_argument"] != node["custom_argument"]
                || entry[UNCENTERED] != node[UNCENTERED]
            {
                conflict = Some(name);
                break;
            }
            children = entry["children"]
                .as_object()
                .expect("working tree children");
        }
        if let Some(name) = conflict {
            problems.push(format!(
                "`{}`: `{name}` is another kind in an earlier usage",
                command.usage
            ));
            continue;
        }
        let mut children = &mut root;
        for (i, (name, node, _)) in nodes.iter().enumerate() {
            let runnable = nodes.get(i + 1).is_none_or(|n| n.2);
            let entry = children.entry(name.clone()).or_insert_with(|| {
                let mut n = node.clone();
                n["executable"] = false.into();
                n["level"] = 4.into();
                n
            });
            if runnable {
                entry["executable"] = true.into();
                let level = entry["execution_level"]
                    .as_u64()
                    .unwrap_or(4)
                    .min(command.level.into());
                entry["execution_level"] = level.into();
            }
            let level = entry["level"]
                .as_u64()
                .unwrap_or(4)
                .min(command.level.into());
            entry["level"] = level.into();
            if !entry["children"].is_object() {
                entry["children"] = Value::Object(Map::new());
            }
            children = entry["children"].as_object_mut().expect("made above");
        }
        let level = levels.entry(nodes[0].0.clone()).or_insert(4);
        *level = (*level).min(command.level);
    }
    let mut root = Value::Object(root);
    finish(&mut root);
    let tree = if levels.is_empty() {
        Value::Null
    } else {
        json!({"type": "root", "children": root})
    };
    (tree, levels, problems)
}

/// Turns the working `level` into the report's `permissions`, and drops
/// empty child lists.
fn finish(children: &mut Value) {
    let Some(map) = children.as_object_mut() else {
        return;
    };
    for node in map.values_mut() {
        let level = node["level"].as_u64().unwrap_or(0) as usize;
        let object = node.as_object_mut().expect("a node");
        object.remove("level");
        if level > 0 {
            object.insert(
                "permissions".into(),
                json!({"type": "require", "permission": {"level": LEVELS[level.min(4)]}}),
            );
        }
        if object
            .get("children")
            .is_some_and(|c| c.as_object().is_some_and(Map::is_empty))
        {
            object.remove("children");
        } else if let Some(c) = object.get_mut("children") {
            finish(c);
        }
    }
}

/// Add the already validated native report tree to the server's dispatcher.
pub(super) fn register_dispatcher(
    dispatcher: &mut Dispatcher,
    tree: &Value,
    handler: &'static str,
    arguments: &CommandArguments,
) {
    fn add(
        dispatcher: &mut Dispatcher,
        parent: usize,
        children: &Map<String, Value>,
        handler: &'static str,
        arguments: &CommandArguments,
    ) {
        for (name, node) in children {
            let id = if node["type"] == "literal" {
                dispatcher.literal(parent, name)
            } else {
                dispatcher.argument(parent, name, argument_type(node, arguments))
            };
            let level = node["permissions"]["permission"]["level"]
                .as_str()
                .and_then(|level| LEVELS.iter().position(|known| *known == level))
                .unwrap_or(0) as u8;
            dispatcher.requires(id, level);
            if node["executable"] == true {
                dispatcher.executes(id, handler);
                dispatcher.requires_execution(
                    id,
                    node["execution_level"]
                        .as_u64()
                        .expect("runnable usage level") as u8,
                );
            }
            if let Some(children) = node["children"].as_object() {
                add(dispatcher, id, children, handler, arguments);
            }
        }
    }
    if let Some(children) = tree["children"].as_object() {
        add(dispatcher, 0, children, handler, arguments);
    }
}

/// The live player body, or a guest record outside a player swap.
pub(super) enum PlayerRead<'a> {
    Acting {
        mode: i32,
        body: Option<&'a PlayerMotion>,
        /// The player's stored abilities, when it is a server player's turn.
        abilities: Option<PlayerAbilities>,
        /// Where the player last died: the acting guest's record, swapped
        /// into the level for their turn.
        last_death: Option<&'a (String, [i32; 3])>,
    },
    Guest(&'a Guest),
}

impl PlayerRead<'_> {
    fn last_death(&self) -> Option<&(String, [i32; 3])> {
        match self {
            Self::Acting { last_death, .. } => *last_death,
            Self::Guest(guest) => guest.last_death.as_ref(),
        }
    }

    fn mode(&self) -> i32 {
        match self {
            Self::Acting { mode, .. } => *mode,
            Self::Guest(guest) => guest.game_mode,
        }
    }

    fn creative(&self) -> bool {
        self.mode() == 1
    }

    fn may_build(&self) -> bool {
        match self {
            Self::Acting {
                abilities: Some(a), ..
            } => a.may_build,
            Self::Guest(guest) => guest.abilities.may_build,
            Self::Acting { mode, .. } => *mode < 2,
        }
    }

    fn body(&self) -> Option<&PlayerMotion> {
        match self {
            Self::Acting { body, .. } => *body,
            Self::Guest(guest) => guest.motion.as_deref(),
        }
    }

    fn food(&self) -> Option<PlayerFood> {
        let food = &self.body()?.health.food;
        let (level, saturation) = match self {
            Self::Acting { .. } => (food.food, food.saturation),
            Self::Guest(guest) => (guest.food, guest.saturation),
        };
        Some(PlayerFood::from(mod_api::__private::built::PlayerFood {
            level,
            saturation,
            exhaustion: food.exhaustion,
        }))
    }

    fn abilities(&self) -> Option<mod_api::PlayerAbilities> {
        let a = match self {
            Self::Acting {
                abilities: Some(a), ..
            } => *a,
            Self::Acting { mode, body, .. } => {
                // No server turn: what the mode gives, with the body's flight.
                let body = body.as_ref()?;
                let mut a = PlayerAbilities {
                    flying: body.flying,
                    flying_speed: body.flying_speed,
                    ..Default::default()
                };
                a.update_for_mode(*mode);
                a.flying = body.flying;
                a
            }
            Self::Guest(guest) => guest.abilities,
        };
        Some(mod_api::PlayerAbilities::from(
            mod_api::__private::built::PlayerAbilities {
                invulnerable: a.invulnerable,
                flying: a.flying,
                may_fly: a.may_fly,
                instabuild: a.instabuild,
                may_build: a.may_build,
                fly_speed: a.flying_speed,
                walk_speed: a.walking_speed,
            },
        ))
    }
}

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Create or retrieve a connectionless player for the level currently in place.
    pub(super) fn fake_player(
        &mut self,
        level: LevelRef,
        uuid: u128,
        name: &str,
    ) -> Option<PlayerRef> {
        if !self.level_in_place(level)
            || !self.lan_guest_swaps.is_empty()
            || !horizon_client::lan::conn::valid_name(name)
        {
            return None;
        }
        let epoch = self.native_lifecycle.epoch()?;
        if self
            .level
            .player_uuid
            .map(horizon_client::lan::playerdata::ints_uuid)
            == Some(uuid)
        {
            return None;
        }
        let host = self.lan.as_ref()?;
        if host
            .hub
            .guests
            .iter()
            .any(|g| g.uuid == uuid && g.participates_online())
        {
            return None;
        }
        if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.dimension == level.dimension_id())
        {
            return (g.name == name).then_some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, g.entity_id as u32),
                uuid,
            ));
        }
        let id = self.next_entity.checked_add(1)?;
        let entity_id = i32::try_from(id).ok()?;
        let motion = PlayerMotion::new(
            self.motion,
            horizon_core::geometry::Vec3::new(0.0, 0.0, 0.0),
        )
        .ok()?;
        let host = self.lan.as_mut()?;
        let conn = host.hub.lan.shared.next_id()?;
        let mut guest = Guest::connectionless(conn, uuid, name.into(), entity_id, [0.0; 3], motion);
        guest.dimension = level.dimension_id().to_owned();
        host.hub.guests.push(guest);
        host.play
            .insert(conn, super::lan_actions::GuestPlay::default());
        self.next_entity = id;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            uuid,
        ))
    }

    /// Discard state belonging to an already removed connectionless guest.
    pub(super) fn cleanup_fake_player(&mut self, host: &mut super::lan::LanHost, guest: &Guest) {
        host.play.remove(&guest.conn);
        host.forget_menus(guest.conn);
        host.poses.remove(&guest.conn);
        host.mob_players.retain(|_, conn| *conn != guest.conn);
        self.destroying.remove(&guest.entity_id);
        self.guest_combat.remove(&guest.conn);
    }

    /// Remove a fake without treating it as a disconnect or saving it.
    pub(super) fn player_remove_fake(&mut self, player: PlayerRef) -> bool {
        if self.native_lifecycle.epoch() != Some(player.fields().entity.epoch())
            || !self.lan_guest_swaps.is_empty()
        {
            return false;
        }
        let Some(mut host) = self.lan.take() else {
            return false;
        };
        let index = host.hub.guests.iter().position(|g| {
            g.is_fake()
                && g.uuid == player.fields().uuid
                && g.entity_id as u32 == player.fields().entity.id()
                && host
                    .hub
                    .holder_guest(g.uuid, &g.dimension)
                    .is_some_and(|owner| owner.conn == g.conn)
        });
        let removed = if let Some(index) = index {
            let guest = host.hub.guests.remove(index);
            self.cleanup_fake_player(&mut host, &guest);
            true
        } else {
            false
        };
        self.lan = Some(host);
        removed
    }

    /// A fake player acts as a player does, through the game's own path for the action
    /// (`ServerGamePacketListenerImpl.handleUseItemOn` / `handleUseItem` / `handleInteract`'s
    /// ATTACK, and `ServerPlayerGameMode.destroyBlock`), as its packet would run it: reach,
    /// protection and the border, then the action. Once it runs, an effect a fake can't take
    /// part in (a menu, sleep, a ride, another dimension) is stopped where it would start, and
    /// the answer says which; the action's own side effects (the swing, the hooks before it,
    /// vanilla's result) have still happened then. The fake is found once, and its turn
    /// starts at that place in the list.
    pub(super) fn player_perform_action(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused> {
        let (gi, conn) = self.fake_action_actor(player, action)?;
        self.perform_fake_action(gi, conn, action)
    }

    /// A machine's one use on the persistent actor, with its private state put back even
    /// when the action refuses. Only the existing guest turn runs gameplay and its hooks.
    pub(super) fn player_perform_action_scoped(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused> {
        let (gi, conn) = self.fake_action_actor(player, action)?;
        if !setup.eye.iter().all(|v| v.is_finite())
            || !setup.yaw.is_finite()
            || !setup.pitch.is_finite()
        {
            return Err(ActionRefused::InvalidSetup);
        }
        let mut state = ScopedPlayerState::new(self.motion, self.items(), setup)?;
        let host = self.lan.as_mut().expect("validated fake's host");
        host.hub.guests.update(gi, |guest| state.swap(guest));
        let play = host.play.insert(conn, Default::default());
        let menus = host.menus.insert(conn, Default::default());
        let outcome = self.perform_fake_action(gi, conn, action);
        let host = self.lan.as_mut().expect("guest turns retain the host");
        let (held, other) = host.hub.guests.update(gi, |guest| {
            let held = guest
                .inventory
                .get_mut(guest.selected)
                .and_then(Option::take)
                .filter(|s| s.count > 0);
            let other: Vec<horizon_items::items::ItemStack> = guest
                .inventory
                .iter_mut()
                .chain([&mut guest.offhand, &mut guest.carried])
                .chain(guest.armor.iter_mut())
                .filter_map(Option::take)
                .filter(|s| s.count > 0)
                .collect();
            state.swap(guest);
            (held, other)
        });
        host.menus.remove(&conn);
        if let Some(menus) = menus {
            host.menus.insert(conn, menus);
        }
        host.play.remove(&conn);
        if let Some(play) = play {
            host.play.insert(conn, play);
        }
        outcome.map(|outcome| {
            ScopedActionResult::new(
                outcome,
                held.try_into_api()
                    .expect("action results use load-resolved item keys"),
                other
                    .try_into_api()
                    .expect("action results use load-resolved item keys"),
            )
        })
    }

    fn fake_action_actor(
        &self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<(usize, u32), ActionRefused> {
        if self.native_lifecycle.epoch() != Some(player.fields().entity.epoch()) {
            return Err(ActionRefused::Stale);
        }
        let found = self.lan.as_ref().and_then(|host| {
            let index = host
                .hub
                .guests
                .identity_index(player.fields().uuid, player.fields().entity.id())?;
            let guest = host.hub.guests.get(index)?;
            guest.placed.then_some((index, guest))
        });
        let (gi, conn) = match found {
            Some((gi, g)) if g.is_fake() => (gi, g.conn),
            Some(_) => return Err(ActionRefused::NotFake),
            // Not in the hub's list as itself: the acting player, or gone.
            None => {
                return Err(match self.player_is_fake(player) {
                    Some(false) => ActionRefused::NotFake,
                    Some(true) => ActionRefused::Busy,
                    None => ActionRefused::Stale,
                });
            }
        };
        if !self.lan_guest_swaps.is_empty()
            || !self.transactions.is_empty()
            || self.fake_action.acting()
        {
            return Err(ActionRefused::Busy);
        }
        if let PlayerAction::InteractOn { location, .. } = action
            && !location.iter().all(|v| v.is_finite())
        {
            return Err(ActionRefused::InvalidSetup);
        }
        if let PlayerAction::Attack { target } | PlayerAction::InteractOn { target, .. } = action
            && (self.native_lifecycle.epoch() != Some(target.epoch())
                || target.id() == player.fields().entity.id())
        {
            return Err(ActionRefused::InvalidTarget);
        }
        Ok((gi, conn))
    }

    fn perform_fake_action(
        &mut self,
        gi: usize,
        conn: u32,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused> {
        let acting = Acting::start(&self.fake_action);
        let failed = std::mem::replace(&mut self.use_failed, false);
        let consumed = (|| match action {
            PlayerAction::UseOn { hand, pos, hit } => {
                let hit = super::Hit {
                    pos,
                    face: hit.face,
                    point: hit.point,
                    inside: hit.inside,
                };
                self.guest_use_item_on(conn, gi, hit, hand)
            }
            PlayerAction::UseItem { hand } => self.guest_use_item(conn, gi, hand, None),
            PlayerAction::InteractOn {
                hand,
                target,
                location,
            } => {
                let id = i32::try_from(target.id()).map_err(|_| ActionRefused::InvalidTarget)?;
                self.guest_interact(conn, gi, id, hand, location)
            }
            PlayerAction::Attack { target } => {
                let id = i32::try_from(target.id()).map_err(|_| ActionRefused::InvalidTarget)?;
                self.guest_attack(conn, gi, id)
            }
            PlayerAction::BreakBlock { pos } => self.guest_break_block(conn, gi, pos),
        })();
        // FAIL ends the click, but is not a consuming InteractionResult.
        let consumed = consumed.map(|used| used && !self.use_failed);
        self.use_failed = failed;
        let consumed = consumed?;
        Ok(match acting.blocked() {
            Some(effect) => ActionOutcome::EffectBlocked(effect),
            None if consumed => ActionOutcome::Consumed,
            None => ActionOutcome::Passed,
        })
    }

    /// Whether `effect` must not start because the player acting is a fake one: true stops it,
    /// and, inside `perform_action`, records it for the action's answer. The one check every
    /// effect a fake can't take part in makes where it starts.
    pub(super) fn fake_blocks(&self, effect: FakeEffect) -> bool {
        if !self.lan_acting_fake {
            return false;
        }
        self.fake_action.block(effect);
        true
    }

    /// Drop the fake players belonging to a level as it closes.
    pub(super) fn remove_level_fake_players(&mut self, dimension: &str) {
        let Some(mut host) = self.lan.take() else {
            return;
        };
        let mut i = 0;
        while i < host.hub.guests.len() {
            let g = &host.hub.guests[i];
            if g.is_fake() && g.dimension == dimension {
                let guest = host.hub.guests.remove(i);
                self.cleanup_fake_player(&mut host, &guest);
            } else {
                i += 1;
            }
        }
        self.lan = Some(host);
    }

    /// Whether a live player handle names a connectionless actor.
    pub(super) fn player_is_fake(&self, player: PlayerRef) -> Option<bool> {
        match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => Some(self.lan_acting_fake),
            PlayerRead::Guest(guest) => Some(guest.is_fake()),
        }
    }

    /// Resolve a real online player by UUID, never a fake. A fake is reached
    /// through the handle returned by its factory. During a nested player swap,
    /// outer players whose records are displaced return None.
    pub(super) fn player(&self, uuid: u128) -> Option<PlayerRef> {
        let epoch = self.native_lifecycle.epoch()?;
        if let Some(remote) = self
            .remote
            .as_ref()
            .filter(|r| r.logged_in && r.profile == uuid)
        {
            return Some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, u32::try_from(remote.entity_id).ok()?),
                uuid,
            ));
        }
        let (_, id) = self.player_lookup(uuid)?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            uuid,
        ))
    }

    /// The chunks players started and stopped watching that the hub
    /// recorded (`lan::chunk_watch`), told to the mods in the hub's order,
    /// with no player's state swapped in, and a sent chunk's passages sent
    /// after it. The player handle is built from the record, so an unwatch
    /// after a disconnect still names them. Nothing while the log is empty.
    pub(super) fn run_chunk_watches(&mut self) {
        use horizon_client::lan::chunk_watch::ChunkWatchKind;
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if host.hub.chunk_watches.is_empty() {
            return;
        }
        let runs = host.hub.chunk_watches.take();
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return;
        };
        for run in runs {
            // A level the server no longer holds: nothing to name.
            let Some(level) = self.level(&run.dimension) else {
                continue;
            };
            for watch in run.watches {
                let Ok(id) = u32::try_from(watch.entity_id) else {
                    continue;
                };
                let player = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, id),
                    watch.uuid,
                );
                let chunk = mod_api::ChunkRef::new(level, watch.chunk);
                match watch.kind {
                    ChunkWatchKind::Watch => self.native_chunk_watch(player, chunk),
                    ChunkWatchKind::Sent => {
                        self.native_chunk_sent(player, chunk);
                        self.send_chunk_passages(level, watch.uuid, watch.chunk);
                    }
                    ChunkWatchKind::Unwatch => self.native_chunk_unwatch(player, chunk),
                }
            }
        }
    }

    /// The level changes the level in place's tickets made since the last
    /// call (`ChunkTickets::take_changes`), told to the mods in order as
    /// `chunk_ticket_level_updated`, with no player's state swapped in. The
    /// tickets keep them only while a mod listens (`update_chunk_tickets`).
    pub(in crate::game) fn run_chunk_ticket_level_updates(&mut self) {
        use horizon_client::ticket_levels::UNLOADED;
        let changes = self.tickets.take_changes();
        if changes.is_empty() {
            return;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return;
        };
        let told = |level| match level {
            UNLOADED => mod_api::ChunkTicketLevel::Unloaded,
            level => mod_api::ChunkTicketLevel::Loaded(level),
        };
        for change in changes {
            let chunk = mod_api::ChunkRef::new(level, [change.chunk.0, change.chunk.1]);
            self.native_chunk_ticket_level_updated(chunk, told(change.old), told(change.new));
        }
    }

    fn player_lookup(&self, uuid: u128) -> Option<(PlayerRead<'_>, u32)> {
        if self.lan_acting_uuid == Some(uuid) && !self.lan_acting_fake {
            return Some((
                PlayerRead::Acting {
                    mode: self.mode_id(),
                    body: self.player.as_ref(),
                    abilities: self.acting_abilities,
                    last_death: self.level.last_death.as_ref(),
                },
                self.lan_acting_entity?,
            ));
        }
        if let Some(guest) = self.lan.as_ref().and_then(|host| {
            host.hub.guests.simulation_iter().find(|guest| {
                guest.placed
                    && guest.participates_online()
                    && guest.uuid == uuid
                    && !self.lan_guest_swaps.contains(&guest.conn)
            })
        }) {
            return Some((PlayerRead::Guest(guest), guest.entity_id as u32));
        }
        if self.lan_acting_uuid.is_none()
            && self.serving.is_some()
            && self
                .level
                .player_uuid
                .map(horizon_client::lan::playerdata::ints_uuid)
                == Some(uuid)
        {
            let body = self.player.as_ref()?;
            return Some((
                PlayerRead::Acting {
                    mode: self.mode_id(),
                    body: Some(body),
                    abilities: self.acting_abilities,
                    last_death: self.level.last_death.as_ref(),
                },
                body.entity_id as u32,
            ));
        }
        None
    }

    /// The dimension id and block of the player's last death, through the
    /// shared player resolver. None for a stale or unavailable player.
    pub(super) fn native_player_last_death_record(
        &self,
        player: PlayerRef,
    ) -> Option<(String, [i32; 3])> {
        if !self.is_logical_server() {
            // A client's copy: what the login and respawn packets carried.
            return (self.local_player()? == player)
                .then(|| self.level.last_death.clone())
                .flatten();
        }
        self.player_handle_read(player)?.last_death().cloned()
    }

    /// `player_handle_read` for the reads that work on both sides. On a
    /// client the local player (`Minecraft.player`) reads its own copy of
    /// what the server sent (`LocalPlayer`): the game mode, food, abilities,
    /// hotbar slot and experience. Any other player reads None there: the
    /// client is not sent their state.
    pub(super) fn player_read_both(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        if self.is_logical_server() {
            return self.player_handle_read(player);
        }
        (self.local_player()? == player).then(|| PlayerRead::Acting {
            mode: self.mode_id(),
            body: self.player.as_ref(),
            abilities: Some(self.client_abilities()),
            last_death: self.level.last_death.as_ref(),
        })
    }
    /// The local player's `Abilities` on a client: what its game mode gives
    /// (`GameType.updatePlayerAbilities`, `mayBuild` among them), then the
    /// flags and speeds `ClientPacketListener.handlePlayerAbilities` took.
    fn client_abilities(&self) -> PlayerAbilities {
        let mut a = PlayerAbilities::default();
        a.update_for_mode(self.mode_id());
        if let Some(p) = self.remote_abilities() {
            a.invulnerable = p.flags & 1 != 0;
            a.flying = p.flags & 2 != 0;
            a.may_fly = p.flags & 4 != 0;
            a.instabuild = p.flags & 8 != 0;
            a.flying_speed = p.flying_speed;
            a.walking_speed = p.walking_speed;
        }
        a
    }

    pub(super) fn player_handle_read(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        (self.native_lifecycle.epoch()? == player.fields().entity.epoch()).then_some(())?;
        if self.lan_acting_fake
            && self.lan_acting_uuid == Some(player.fields().uuid)
            && self.lan_acting_entity == Some(player.fields().entity.id())
        {
            return Some(PlayerRead::Acting {
                mode: self.mode_id(),
                body: self.player.as_ref(),
                abilities: self.acting_abilities,
                last_death: self.level.last_death.as_ref(),
            });
        }
        if let Some((read, id)) = self.player_lookup(player.fields().uuid)
            && id == player.fields().entity.id()
        {
            return Some(read);
        }
        let guests = &self.lan.as_ref()?.hub.guests;
        let guest = guests
            .get(guests.identity_index(player.fields().uuid, player.fields().entity.id())?)?;
        if !guest.placed || !guest.is_fake() || self.lan_guest_swaps.contains(&guest.conn) {
            return None;
        }
        Some(PlayerRead::Guest(guest))
    }

    /// `ServerPlayerGameMode.getGameModeForPlayer`; Paper `HumanEntity#getGameMode`.
    /// None for stale, offline or unavailable players (including displaced outer players).
    pub(super) fn player_game_mode(&self, player: PlayerRef) -> Option<GameMode> {
        match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => Some(self.mode),
            PlayerRead::Guest(guest) => GameMode::from_id(guest.game_mode),
        }
    }

    /// Changes a player's game mode as `ServerPlayer.setGameMode` and
    /// `/gamemode` do, including the cancellable change hook and ability
    /// updates. Returns false for a stale or unavailable player, an active
    /// outer guest, no change, or a canceled change.
    pub(super) fn player_set_game_mode(&mut self, player: PlayerRef, mode: GameMode) -> bool {
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut changed = false;
        self.native_as_player_handle(player, |game| {
            changed = game.set_game_mode(mode);
        }) && changed
    }

    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. A speed that is not finite refuses the write
    /// (vanilla's setters take any float). Returns false for a
    /// stale or unavailable player, a displaced outer player, and a speed
    /// that is not finite.
    pub(super) fn player_set_abilities(
        &mut self,
        player: PlayerRef,
        abilities: mod_api::PlayerAbilities,
    ) -> bool {
        if !abilities.fly_speed.is_finite() || !abilities.walk_speed.is_finite() {
            return false;
        }
        let stored = PlayerAbilities {
            invulnerable: abilities.invulnerable,
            flying: abilities.flying,
            may_fly: abilities.may_fly,
            instabuild: abilities.instabuild,
            may_build: abilities.may_build,
            flying_speed: abilities.fly_speed,
            walking_speed: abilities.walk_speed,
        };
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut stored_here = false;
        self.native_as_player_handle(player, |game| {
            let Some(slot) = game.acting_abilities.as_mut() else {
                return;
            };
            *slot = stored;
            if let Some(body) = game.player.as_mut() {
                body.invulnerable = stored.invulnerable;
                body.may_fly = stored.may_fly;
                body.flying = stored.flying;
                body.flying_speed = stored.flying_speed;
            }
            stored_here = true;
        }) && stored_here
    }

    /// `Player.isCreative`; Paper `HumanEntity#getGameMode`.
    pub(super) fn player_is_creative(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.creative())
    }

    /// `Inventory.getSelectedSlot`: the hotbar slot the player holds, 0..9.
    pub(super) fn player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => self.selected,
            PlayerRead::Guest(guest) => guest.selected,
        })
    }

    /// `Inventory.setSelectedSlot` for a server write: `slot` (0 to 8) becomes the hotbar slot the
    /// player holds and a guest's client is told (`SetHeldSlot`). A new slot ends a main-hand use,
    /// as `handleSetCarriedItem` does, without `player_switch_hotbar_slot_pre`, which is for the
    /// player's own input. False for a slot past 8, a stale or dead player.
    pub(super) fn player_set_selected_slot(&mut self, player: PlayerRef, slot: usize) -> bool {
        if slot >= 9 {
            return false;
        }
        // The connection to tell: a parked guest's, or the acting guest's, whose state is in
        // place for their turn (`leave_guest` stores the slot back but sends nothing). The host
        // has none.
        let conn = match self.player_handle_read(player) {
            Some(PlayerRead::Guest(guest)) => Some(guest.conn),
            Some(PlayerRead::Acting { .. }) => self.lan_acting_uuid.and_then(|uuid| {
                self.lan
                    .as_ref()?
                    .hub
                    .guests
                    .iter()
                    .find(|guest| guest.uuid == uuid)
                    .map(|guest| guest.conn)
            }),
            None => None,
        };
        let changed = self.player_selected_slot(player) != Some(slot);
        if !self.native_as_player_handle(player, |game| {
            if changed
                && game
                    .using
                    .is_some_and(|(held, _)| held != crate::game::shield::OFFHAND_SLOT)
            {
                game.stop_active_item_use();
            }
            game.selected = slot;
        }) {
            return false;
        }
        if let (Some(conn), Some(host)) = (conn, self.lan.as_ref()) {
            host.hub.send_to(
                conn,
                &C::SetHeldSlot(play::SetHeldSlot { slot: slot as i32 }),
            );
        }
        true
    }

    /// `Player.drop(stack, false, thrown_by_player)`: `stack` thrown from the player's eyes along
    /// their view as an item entity with the drop key's pickup delay, the main hand swinging and
    /// `item_toss` asked first. Nothing is taken from an inventory: the stack is the caller's
    /// (take it out through the item places first). `thrown_by_player` names the player as the
    /// thrower, as the drop key does. None once the item is in the world; the stack back,
    /// untouched, for an empty stack, a stale or dead player and a toss a mod canceled, so a
    /// refused drop never loses items.
    pub(super) fn player_drop_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
        thrown_by_player: bool,
    ) -> Option<ItemStack> {
        if self.entity_ids_exhausted() {
            return Some(stack);
        }
        if stack.count == 0 {
            return Some(stack);
        }
        // `throw_from_eyes` discards a vetoed toss, as `item_toss` always has: keep a copy so
        // the caller gets it back.
        let copy = stack.clone();
        let mut dropped = false;
        let ran = self.native_as_player_handle(player, |game| {
            dropped = game.drop_stack_from_eyes(stack, thrown_by_player).is_some();
        });
        (!(ran && dropped)).then_some(copy)
    }

    /// `Player.isSpectator`; Paper `HumanEntity#getGameMode`.
    pub(super) fn player_is_spectator(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.mode() == 3)
    }

    /// `Player.mayBuild`; Paper `HumanEntity#getGameMode`.
    pub(super) fn player_may_build(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.may_build())
    }

    /// `FoodData.getFoodLevel/getSaturationLevel/getExhaustionLevel`; Paper
    /// `HumanEntity#getFoodLevel/getSaturation/getExhaustion`. None until the
    /// guest's body exists, as well as for stale or unavailable players.
    pub(super) fn player_food(&self, player: PlayerRef) -> Option<PlayerFood> {
        self.player_read_both(player)?.food()
    }

    /// `Player.getAbilities`; Paper `Player#getAllowFlight/isFlying/getFlySpeed/getWalkSpeed`.
    pub(super) fn player_abilities(&self, player: PlayerRef) -> Option<mod_api::PlayerAbilities> {
        self.player_read_both(player)?.abilities()
    }

    /// The players on this server, in the order they joined. Empty on a
    /// client and in a world nobody else can join.
    pub(super) fn native_server_players(&self) -> Vec<PlayerRef> {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return Vec::new();
        };
        let Some(host) = &self.lan else {
            return Vec::new();
        };
        // Construct from the online records, not the live-body resolver: outer
        // players remain connected while their bodies are displaced by a turn.
        host.hub
            .online_players()
            .map(|g| {
                mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, g.entity_id as u32),
                    g.uuid,
                )
            })
            .collect()
    }

    /// Find a connected player by their account name, case-insensitively.
    pub(super) fn native_player_by_name(&self, name: &str) -> Option<PlayerRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let player = self
            .lan
            .as_ref()?
            .hub
            .online_players()
            .find(|player| player.name.eq_ignore_ascii_case(name))?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, player.entity_id as u32),
            player.uuid,
        ))
    }

    /// A connected player's record, including one displaced by a nested turn.
    pub(super) fn online_player_record(&self, player: PlayerRef) -> Option<&Guest> {
        (self.native_lifecycle.epoch()? == player.entity().epoch()).then_some(())?;
        self.lan
            .as_ref()?
            .hub
            .online_players()
            .find(|g| g.uuid == player.uuid() && g.entity_id as u32 == player.entity().id())
    }

    /// The stable name of a connected or acting player. None for a stale or
    /// unavailable player; displaced online players retain their identity.
    pub(super) fn player_name(&self, player: PlayerRef) -> Option<String> {
        if self.native_acting_player() == Some(player) {
            return self.lan_acting_name.clone();
        }
        Some(self.online_player_record(player)?.name.clone())
    }

    /// The dimension key of an online player (`ServerPlayer.level().dimension()`):
    /// the dimension the player is placed in.
    pub(super) fn player_dimension_key(&self, player: PlayerRef) -> Option<mod_api::DimensionKey> {
        let epoch = self.native_lifecycle.epoch()?;
        if epoch != player.fields().entity.epoch() {
            return None;
        }
        if !self.is_logical_server() {
            return (self.local_player()? == player).then(|| self.native_level_key());
        }
        if self.native_acting_player() == Some(player) {
            let dim = match self.lan_acting {
                Some(conn) => {
                    let g = self.command_guest(conn)?;
                    Dim::from_id(&g.dimension).key()
                }
                None => self.dim.key(),
            };
            return Some(dim);
        }
        if let Some(guest) = self.online_player_record(player) {
            return Some(Dim::from_id(&guest.dimension).key());
        }
        if self.serving.is_some()
            && self.player(player.uuid()) == Some(player)
            && self
                .level
                .player_uuid
                .map(horizon_client::lan::playerdata::ints_uuid)
                == Some(player.uuid())
        {
            return Some(self.dim.key());
        }
        None
    }

    /// The level the online player is in (`ServerPlayer.level`): on an integrated
    /// server or dedicated server, the player's current dimension; on a client,
    /// the client's level. None for a stale or unavailable player.
    pub(super) fn player_level(&self, player: PlayerRef) -> Option<LevelRef> {
        let dim = self.player_dimension_key(player)?;
        Some(self.level_ref(player.fields().entity.epoch(), dim))
    }

    /// The dimension the online player is in (`ServerPlayer.level().dimension()`):
    /// `minecraft:overworld`, `minecraft:the_nether`, ... None for a stale
    /// or unavailable player.
    pub(super) fn player_dimension(&self, player: PlayerRef) -> Option<Identifier> {
        let level = self.player_level(player)?;
        Identifier::parse(level.dimension_id()).ok()
    }

    /// The IP address the player connects from, without the port (behind a
    /// Velocity proxy, the address the proxy forwards). Displaced connected
    /// players keep their address; an acting player with no known connection
    /// has an empty address. None for a stale or unavailable player.
    pub(super) fn player_address(&self, player: PlayerRef) -> Option<String> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.address.ip().to_string());
        }
        (self.native_acting_player() == Some(player)).then(String::new)
    }

    /// The brand the player's client announced when it connected (`ServerPlayer.clientBrand`),
    /// empty when it sent none. Displaced connected players keep theirs; an acting player with
    /// no known connection has an empty one. None for a stale or unavailable player.
    pub(super) fn player_client_brand(&self, player: PlayerRef) -> Option<String> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.brand.clone());
        }
        (self.native_acting_player() == Some(player)).then(String::new)
    }

    /// The plugin channels the player's client announced (`minecraft:register` less
    /// `minecraft:unregister`), kept by its connection from configuration on.
    pub(super) fn player_announced_channels(&self, player: PlayerRef) -> Option<Vec<String>> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.sender.announced.list());
        }
        (self.native_acting_player() == Some(player)).then(Vec::new)
    }

    /// Validate identity without requiring a live body: sending to an online
    /// displaced player still works, as it did through the UUID send path.
    fn message_player_uuid(&self, player: PlayerRef) -> Option<u128> {
        (self.native_acting_player() == Some(player) || self.online_player_record(player).is_some())
            .then_some(player.uuid())
    }
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    pub(super) fn player_tell(&mut self, player: PlayerRef, line: Text) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        self.native_send(uuid, system_chat(&line))
    }
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on.
    pub(super) fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let packet = C::SystemChat(play::SystemChat {
            content: component_nbt(&line.component()),
            overlay: true,
        });
        self.native_send(uuid, packet)
    }
    /// Sends `packet` to that player, also when they are the acting player
    /// (their packets then go after the hook); false when they aren't on.
    pub(super) fn native_send(&mut self, uuid: u128, packet: C) -> bool {
        self.native_send_all(uuid, vec![packet])
    }
    fn native_send_all(&mut self, uuid: u128, packets: Vec<C>) -> bool {
        if self.lan_acting_uuid == Some(uuid) && !self.lan_acting_fake {
            self.guest_packets.extend(packets);
            return true;
        }
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.online_players().find(|g| g.uuid == uuid) else {
            return false;
        };
        for packet in &packets {
            host.hub.send_to(g.conn, packet);
        }
        true
    }
    /// Shows `sidebar` on that player's screen, in place of the one shown
    /// before, or takes it away (None). Only their client holds it: the
    /// server's scoreboard, its `/scoreboard` and the other players don't
    /// see it, and it is gone when they leave (a mod sends it again when
    /// they come back). It takes the sidebar's place from an objective the
    /// world shows there, for this player, until they rejoin. False when
    /// they aren't on.
    pub(super) fn native_sidebar(&mut self, uuid: u128, sidebar: Option<&NativeSidebar>) -> bool {
        let objective = SIDEBAR_OBJECTIVE.to_owned();
        // Removed first, so the new one's lines replace all the old ones.
        let mut packets = vec![C::SetObjective(play::SetObjective {
            name: objective.clone(),
            method: 1,
            display: None,
        })];
        if let Some(sidebar) = sidebar {
            let title = component_nbt(&sidebar.title.component());
            packets.push(C::SetObjective(play::SetObjective {
                name: objective.clone(),
                method: 0,
                display: Some((title, 0, Some(play::NumberFormat::Blank))),
            }));
            // DisplaySlot.SIDEBAR.
            packets.push(C::SetDisplayObjective(play::SetDisplayObjective {
                slot: 1,
                objective: objective.clone(),
            }));
            let lines = &sidebar.lines[..sidebar.lines.len().min(15)];
            for (i, (left, right)) in lines.iter().enumerate() {
                packets.push(C::SetScore(play::SetScore {
                    // Highest first. The display name shows, not the owner;
                    // an owner starting with '#' is hidden from the sidebar
                    // (ScoreHolder's isHidden), and ':' is in no player's name.
                    owner: format!("horizon:line{i:02}"),
                    objective: objective.clone(),
                    score: (lines.len() - i) as i32,
                    display: Some(component_nbt(&left.component())),
                    number_format: Some(match right {
                        Some(t) => play::NumberFormat::Fixed(component_nbt(&t.component())),
                        None => play::NumberFormat::Blank,
                    }),
                }));
            }
        }
        self.native_send_all(uuid, packets)
    }
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on.
    pub(super) fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let packet = C::TabList(play::TabList {
            header: component_nbt(&header.component()),
            footer: component_nbt(&footer.component()),
        });
        self.native_send(uuid, packet)
    }
    /// The server's tick rate (20 unless `/tick rate` changed it) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower.
    pub(super) fn tick_stats(&self) -> (f32, f64) {
        (
            self.clock.rate(),
            self.clock.average_nanos() as f64 / 1_000_000.0,
        )
    }
    /// A mod's `set_tick_rate`: `apply_tick_rate`, refused inside the `server_tick_rate_changed`
    /// hook so a listener can't fire it again without end.
    pub(in crate::game) fn set_tick_rate(&mut self, rate: f32) -> bool {
        !LevelControl::TickRate.is_heard() && self.apply_tick_rate(rate)
    }
    /// `/tick rate`'s `ServerTickRateManager.setTickRate`, then `MinecraftServer.onTickRateChanged`
    /// (the hook, and the autosave wait shortened). False for a rate outside 1 to 10000 and
    /// on a client, which follows its server's rate. The command runs it from a listener too,
    /// as vanilla does.
    pub(in crate::game) fn apply_tick_rate(&mut self, rate: f32) -> bool {
        if !self.is_logical_server() || !(1.0..=10_000.0).contains(&rate) {
            return false;
        }
        self.clock.set_rate(rate);
        self.on_tick_rate_changed();
        true
    }
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. False when the server list is unavailable, the handle is
    /// stale or the player isn't online.
    pub(super) fn player_set_sleep_ignored(&mut self, player: PlayerRef, ignored: bool) -> bool {
        let Some(uuid) = self.sleep_list_uuid(player) else {
            return false;
        };
        let Some(host) = &mut self.lan else {
            return false;
        };
        if ignored {
            host.sleep_ignored.insert(uuid);
        } else {
            host.sleep_ignored.remove(&uuid);
        }
        true
    }
    /// The UUID of a player the server list holds, whom the sleep count can
    /// leave out: None when the list is unavailable, the handle is stale or
    /// the player isn't online.
    fn sleep_list_uuid(&self, player: PlayerRef) -> Option<u128> {
        let uuid = self.message_player_uuid(player)?;
        self.lan
            .as_ref()?
            .hub
            .online_players()
            .any(|g| g.uuid == uuid)
            .then_some(uuid)
    }
    /// Whether `player_set_sleep_ignored` left the player out of the sleep
    /// count. None as the write's false.
    pub(super) fn player_sleep_ignored(&self, player: PlayerRef) -> Option<bool> {
        let uuid = self.sleep_list_uuid(player)?;
        Some(self.lan.as_ref()?.sleep_ignored.contains(&uuid))
    }
    /// Whether `SleepStatus.update` counts the player: not a spectator, not a
    /// fake, and not an ignored player who is out of bed (`sleep_counts`'s
    /// own rule). None for a stale or unavailable player.
    pub(super) fn player_counts_for_sleep(&self, player: PlayerRef) -> Option<bool> {
        let ignored = |game: &Self| {
            game.lan
                .as_ref()
                .is_some_and(|host| host.sleep_ignored.contains(&player.uuid()))
        };
        Some(match self.player_handle_read(player)? {
            // The player in place: the host's own, or a guest in their turn.
            PlayerRead::Acting { .. } => {
                !self.lan_acting_fake
                    && !self.spectator()
                    && (self.sleeping.is_some() || !ignored(self))
            }
            PlayerRead::Guest(guest) => self.guest_counts_for_sleep(guest),
        })
    }
    /// `Player.getSleepTimer`: the ticks in bed (`sleepCounter`), in the
    /// record of a guest or in place for the acting player. None for a
    /// stale or unavailable player.
    pub(super) fn player_sleep_timer(&self, player: PlayerRef) -> Option<u32> {
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.sleep_timer,
            PlayerRead::Guest(guest) => guest.sleep_timer,
        })
    }
    /// Sets `Player.sleepCounter` to `ticks`, 0 to 109 (what `tick_sleep_timer`
    /// reaches), through the sleep writes' guard: false for a fake player,
    /// a guest in another level and a stale player.
    pub(super) fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: u32) -> bool {
        if ticks > super::MAX_SLEEP_TIMER || self.player_handle_read(player).is_none() {
            return false;
        }
        let Some(target) = self.native_entity_target(player.entity()) else {
            return false;
        };
        self.native_player_sleep_write(target, |game| {
            game.sleep_timer = ticks;
            true
        })
    }
    /// `ServerPlayer.isTextFilteringEnabled`: what the client's
    /// `ClientInformation` said, false for the player running the server and
    /// for an acting player with no connection. None for a stale or
    /// unavailable player.
    pub(super) fn player_text_filtering(&self, player: PlayerRef) -> Option<bool> {
        if let Some(guest) = self.online_player_record(player) {
            return Some(guest.info.text_filtering);
        }
        (self.native_acting_player() == Some(player)).then_some(false)
    }
    /// `ServerPlayer.getRespawnConfig`: the player's respawn point as the
    /// game keeps it (the guest's record, or the acting player's in place),
    /// `Some(None)` when they have none. None for a stale or unavailable
    /// player, or a dimension key the API can't name.
    pub(super) fn player_respawn_config(
        &self,
        player: PlayerRef,
    ) -> Option<Option<PlayerRespawnConfig>> {
        let saved = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.respawn_config.map(|c| c.saved()),
            PlayerRead::Guest(guest) => guest.respawn_elsewhere.clone(),
        };
        let Some((dimension, [x, y, z], yaw, _pitch, forced)) = saved else {
            return Some(None);
        };
        Some(Some(PlayerRespawnConfig {
            position: GlobalPos::new(Identifier::parse(&dimension).ok()?, BlockPos::new(x, y, z)),
            yaw,
            forced,
        }))
    }
    /// `ServerPlayer.setRespawnPosition`, as `/spawnpoint` sets it (the
    /// same `set_respawn_point`): `config`, or none for None. False for a
    /// stale, unavailable or dead player, a dimension the game doesn't
    /// have, a yaw that isn't finite and a change a mod canceled.
    pub(super) fn player_set_respawn_config(
        &mut self,
        player: PlayerRef,
        config: Option<PlayerRespawnConfig>,
    ) -> bool {
        let stored = match config {
            Some(config) => {
                let dimension = config.position.dimension().to_string();
                if !config.yaw.is_finite() || !Dim::exists(&dimension) {
                    return false;
                }
                let at = config.position.pos();
                Some(RespawnConfig::wrapped(
                    Dim::from_id(&dimension),
                    [at.x, at.y, at.z],
                    config.yaw,
                    config.forced,
                ))
            }
            None => None,
        };
        let mut set = false;
        self.native_as_player_handle(player, |game| set = game.set_respawn_point(stored)) && set
    }
    /// Hides that player (a vanish) from the players below permission
    /// `level`, or shows them again (None): from the next tick those players
    /// lose them from the Tab list and the world (their client stops
    /// tracking the player, as it does a spectator), and don't read their
    /// joining or leaving; players of `level` or more still see them, and
    /// a player made an operator meanwhile sees them again. Kept by UUID, even
    /// after this handle disconnects (the player rejoins hidden), until a mod sets it again
    /// or the server stops. Works inside a player's hook too; false for a stale
    /// world epoch, on a client and in a world nobody else can join.
    pub(super) fn player_hide(&mut self, player: PlayerRef, level: Option<u8>) -> bool {
        if self.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return false;
        }
        let uuid = player.uuid();
        let Some(host) = &mut self.lan else {
            return false;
        };
        match level {
            Some(level) => host.hub.hidden.insert(uuid, level),
            None => host.hub.hidden.remove(&uuid),
        };
        true
    }
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (`Messages::text_allowed`, its length,
    /// other players' names). False when the server list is unavailable,
    /// the handle is stale or the player isn't online.
    pub(super) fn player_set_nickname(
        &mut self,
        player: PlayerRef,
        nickname: Option<&str>,
    ) -> bool {
        let Some(uuid) = self.message_player_uuid(player) else {
            return false;
        };
        let Some(host) = &mut self.lan else {
            return false;
        };
        if !host.hub.online_players().any(|g| g.uuid == uuid) {
            return false;
        }
        host.hub.set_nickname(uuid, nickname.map(str::to_owned));
        true
    }
    /// Whether the server's chat filter (`server.toml` `[chat_filter]`)
    /// lets `text` through: true without a filter, None inside a player's
    /// hook (where the server is out of reach).
    pub(super) fn native_text_allowed(&self, text: &str) -> Option<bool> {
        let host = self.lan.as_ref()?;
        Some(
            host.hub
                .chat_filter
                .as_ref()
                .is_none_or(|g| !g.matches(text)),
        )
    }
    /// A system line in every player's chat and the server's log.
    pub(super) fn native_broadcast(&self, mut line: Text) {
        let mut cancel = false;
        self.native_broadcast_message(&mut line, &mut cancel);
        if cancel {
            return;
        }
        let plain = line.plain();
        if self.is_dedicated_server() {
            super::dedicated::log("INFO", &plain);
        } else {
            log::info!("{plain}");
        }
        let Some(host) = &self.lan else {
            return;
        };
        let packet = system_chat(&line);
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            host.hub.send_to(g.conn, &packet);
        }
    }
    /// Posts a line to the dedicated server's event stream (`GET /api/v1/events`)
    /// as an `announce` event, which horizon-server's Discord bridge relays when
    /// `[discord] events` lists `announce`: `name` and `uuid` say who it is
    /// about (empty and 0 for no one), `message` is the line (`§` codes colour it).
    /// Only on a dedicated server; false elsewhere and for an empty message.
    pub(super) fn native_server_event(&self, name: &str, uuid: u128, message: &str) -> bool {
        if !self.is_dedicated_server() || message.trim().is_empty() {
            return false;
        }
        super::dedicated::events::record("announce", name, uuid, json!({ "message": message }));
        true
    }
    /// A command's success feedback, routed to its originating source using
    /// vanilla's silence, send-command-feedback and block/cart track-output rules.
    pub(super) fn native_feedback(&mut self, line: Text) {
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        self.with_source(
            super::commands::execute::Source {
                dimension: source.origin_dimension,
                ..source
            },
            |game| game.feedback_unbroadcast(line),
        );
    }
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (`native_run_mod_command`, `native_right_click_block`,
    /// `native_use_on`, `native_break_block`...): None outside such a
    /// hook.
    pub(super) fn native_acting_player(&self) -> Option<PlayerRef> {
        self.player.as_ref()?;
        self.lan_acting_name.as_ref()?;
        self.lan_acting_permission?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(self.native_lifecycle.epoch()?, self.native_acting_entity()),
            self.lan_acting_uuid?,
        ))
    }
    /// Runs `line` (no slash) as the server's console does: permission
    /// level 4, from the overworld's spawn, with no output. A teleport to
    /// another level (`execute in minecraft:the_nether run tp Alex 0 70 0`)
    /// happens at the next tick's start, as the console's. Whether it ran
    /// without failing. During a guest's turn, live players (including the
    /// acting player) are reachable; displaced outer players are excluded
    /// from name, UUID and selector resolution on this path only.
    pub(super) fn native_run_command(&mut self, line: &str) -> bool {
        if self.lan.is_none() {
            return false;
        }
        let server = execute::Source {
            dimension: Dim::Overworld,
            ..self.server_source()
        };
        let outer = self.command_source.replace(server);
        let filter = std::mem::replace(&mut self.native_command_filter, true);
        let outer_exec = std::mem::take(&mut self.exec);
        let ok = self.as_console(|g| g.run_command(line));
        self.exec = outer_exec;
        self.native_command_filter = filter;
        self.command_source = outer;
        ok
    }
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    ///
    /// Not on wasm: a path in the game's file system, which a wasm mod has none of; a wasm mod
    /// keeps world data in level attachments.
    pub(super) fn world_dir(&self) -> Option<PathBuf> {
        (!self.world_root.as_os_str().is_empty()).then(|| self.world_root.clone())
    }
}

/// Whether an orb at `position` (its box: 0.5 wide and high, standing on
/// it) meets the box `min`..`max`, touching faces excluded (`AABB.intersects`).
fn orb_meets(position: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    position[0] - 0.25 < max[0]
        && position[0] + 0.25 > min[0]
        && position[1] < max[1]
        && position[1] + 0.5 > min[1]
        && position[2] - 0.25 < max[2]
        && position[2] + 0.25 > min[2]
}

/// The box `min`..`max` with each axis's pair in order, as vanilla's `AABB` takes it;
/// None when a bound is not finite (NaN or infinite).
fn ordered_box(min: [f64; 3], max: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    if !min.iter().chain(&max).all(|v| v.is_finite()) {
        return None;
    }
    Some((
        std::array::from_fn(|i| min[i].min(max[i])),
        std::array::from_fn(|i| min[i].max(max[i])),
    ))
}

/// Experience for server mods.
#[allow(dead_code)]
impl Game<'_> {
    /// The experience orbs of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: what a mod reads before it
    /// cancels an orb in `native_entity_join_level`, to give its points with
    /// `PlayerRef::give_experience_points`. Empty for a level not in place, on a
    /// client of a server and for a bound that is not finite (NaN or infinite); a
    /// minimum above its maximum is the same box with the two swapped (`AABB`).
    pub(super) fn experience_orbs(
        &self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Vec<mod_api::ExperienceOrb> {
        let Some((min, max)) = ordered_box(min, max) else {
            return Vec::new();
        };
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return Vec::new();
        }
        self.orbs
            .iter()
            .filter(|o| o.count > 0)
            .map(|o| mod_api::ExperienceOrb {
                id: o.id,
                position: [o.position.x, o.position.y, o.position.z],
                value: o.value,
                count: o.count,
            })
            .filter(|o| orb_meets(o.position, min, max))
            .collect()
    }
    /// Writes the experience orb `id`'s `value` (the points a pickup gives), `count`
    /// (orbs merged into it, each worth `value`) and `age` (ticks lived; an orb
    /// despawns at 6000); a `None` is left as it is.
    /// False, and nothing written, for an orb not in the server level `dimension`, a
    /// `count` of 0 or above `i32::MAX`, a `value` outside -32768..=32767 (a save keeps
    /// `Value` as a short and `Count` as an int, so a write it accepts reloads
    /// unchanged), and on a client of a server. In `experience_orb_merge`, a write to
    /// `into` is the merged orb.
    pub(super) fn set_experience_orb(
        &mut self,
        dimension: &str,
        id: u32,
        value: Option<i32>,
        count: Option<u32>,
        age: Option<u32>,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || count == Some(0)
            || count.is_some_and(|c| c > crate::orb_limits::MAX_COUNT)
            || value.is_some_and(|v| !crate::orb_limits::value_fits(v))
        {
            return false;
        }
        let Some(orb) = self.orbs.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        orb.value = value.unwrap_or(orb.value);
        orb.count = count.unwrap_or(orb.count);
        orb.age = age.unwrap_or(orb.age);
        true
    }
    pub(super) fn native_as_player_handle(
        &mut self,
        player: PlayerRef,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        let Some(read) = self.player_handle_read(player) else {
            return false;
        };
        let conn = match read {
            PlayerRead::Acting { .. } => None,
            PlayerRead::Guest(g) if !g.dead => Some(g.conn),
            PlayerRead::Guest(_) => return false,
        };
        let Some(conn) = conn else {
            if self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0)
            {
                return false;
            }
            f(self);
            return true;
        };
        self.as_guest(conn, f).is_some()
    }

    /// Runs `f` with the player `uuid`'s state in place: theirs already when
    /// they are the acting player (the host, or a guest swapped in), else a
    /// guest's through `as_guest`, including within another guest's turn.
    /// False when it didn't run: no such player here, or dead.
    pub(super) fn native_as_player(&mut self, uuid: u128, f: impl FnOnce(&mut Self)) -> bool {
        if uuid == self.native_acting_id() {
            if self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0)
            {
                return false;
            }
            f(self);
            return true;
        }
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let conn = host
            .hub
            .holder_guest(uuid, self.dim.id())
            .filter(|g| !g.dead)
            .map(|g| g.conn);
        conn.and_then(|conn| self.as_guest(conn, f)).is_some()
    }
}

pub(super) fn system_chat(line: &Text) -> C {
    C::SystemChat(play::SystemChat {
        content: component_nbt(&line.component()),
        overlay: false,
    })
}

/// Native command metadata and the server dispatcher containing their parsed usages.
pub(super) struct NativeCommands {
    levels: HashMap<String, u8>,
    tree: Value,
    arguments: CommandArguments,
}

/// Where the game calls the server hook points.
impl Game<'_> {
    /// Asks the mods for their commands once per hub, and puts them in every
    /// player's command tree; again when a mod asked (`native_refresh_commands`).
    pub(super) fn register_native_commands(&mut self) {
        let refresh = std::mem::take(&mut self.native_commands_refresh);
        if self
            .lan
            .as_ref()
            .is_none_or(|host| host.native_commands.is_some() && !refresh)
        {
            return;
        }
        let mut commands = Vec::new();
        self.native_register_commands(&mut commands);
        let vanilla = Arc::new(horizon_client::command::vanilla::session_dispatcher(
            &self.level.rule_registry,
            &self.level.enabled_features,
        ));
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        // Horizon's own moderation commands go in the same tree, so operators' clients complete them
        // instead of showing them as unknown; the game still runs them itself (`run_guest_command`).
        let extras = super::tempban::moderation_commands(
            host.hub.chat_filter_on(),
            self.server_lists.is_some(),
        );
        commands.extend(extras.iter().cloned());
        let arguments = command_arguments();
        let (tree, mut levels, problems) = command_tree(
            &commands,
            |name| host.hub.vanilla_command(name) || vanilla.find(name).is_some(),
            &arguments,
        );
        for p in problems {
            log::warn!("Horizon: a native mod's command was left out: {p}");
        }
        levels.retain(|name, _| {
            !extras
                .iter()
                .any(|c| c.usage.split(' ').next() == Some(name.as_str()))
        });
        let mut execution_tree = tree.clone();
        if let Some(children) = execution_tree["children"].as_object_mut() {
            children.retain(|name, _| levels.contains_key(name));
        }
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.native_commands = Some(NativeCommands {
            levels,
            tree: execution_tree,
            arguments,
        });
        host.hub
            .set_selector_tokens(super::native_entity_selectors::selector_tokens());
        if refresh {
            host.hub.store_mod_commands(0, tree);
        } else {
            host.hub.set_mod_commands(0, tree);
        }
        self.ui.dispatcher.0 = vanilla;
        self.restore_native_commands();
        if refresh {
            self.resend_commands_filtered();
        }
    }

    /// Restore mod nodes after vanilla rebuilds the session's command tree.
    pub(super) fn restore_native_commands(&mut self) {
        if let Some(commands) = self
            .lan
            .as_ref()
            .and_then(|host| host.native_commands.as_ref())
        {
            register_dispatcher(
                Arc::make_mut(&mut self.ui.dispatcher.0),
                &commands.tree,
                "native_mod_command",
                &commands.arguments,
            );
        }
    }

    /// A mod changed the commands it registers (`native_register_commands`):
    /// at the start of the next server tick every mod is asked again, the
    /// commands players may run are rebuilt, and every player on the server is
    /// sent their tree again, after `native_player_commands_sent`. Nothing
    /// happens where no world is open to players, and nothing for a server
    /// whose mods never call it. API @API:refresh_commands@.
    pub(super) fn native_refresh_commands(&mut self) {
        self.native_commands_refresh = true;
    }

    /// Asks the mods what root commands each player's tree keeps
    /// (`native_player_commands_sent`) and stores the answer in the hub,
    /// which sends the trees without what the mods took out: for `only`, or
    /// for every placed player.
    pub(super) fn native_filter_commands(&mut self, only: Option<u32>) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let players: Vec<(u32, u128, String)> = host
            .hub
            .guests
            .iter()
            .filter(|g| only.map_or(g.placed, |conn| g.conn == conn))
            .map(|g| (g.conn, g.uuid, g.name.clone()))
            .collect();
        for (conn, uuid, name) in players {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let names = host.hub.command_names(conn);
            let mut kept = names.clone();
            let placed = host.hub.guest(conn).is_some_and(|g| g.placed);
            if !placed {
                // A joining player has no world yet: nothing can be told them.
                self.native_player_commands_sent(uuid, &name, &mut kept);
            } else if self.lan_acting == Some(conn) {
                self.native_player_commands_sent(uuid, &name, &mut kept);
            } else {
                self.as_guest(conn, |g| {
                    g.native_player_commands_sent(uuid, &name, &mut kept);
                });
            }
            if let Some(host) = self.lan.as_mut() {
                let _ = host.hub.update_guest(conn, |g| {
                    g.hidden_commands = names.into_iter().filter(|n| !kept.contains(n)).collect();
                });
            }
        }
    }

    /// `Hub::resend_commands` with the mods' say on each tree.
    pub(super) fn resend_commands_filtered(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.hub.refresh_permissions();
        self.native_filter_commands(None);
        if let Some(host) = self.lan.as_mut() {
            host.hub.resend_commands();
        }
    }

    /// The traversal level of a mod's root (`line`, without its slash), when registered.
    /// The parsed executable node enforces its own usage level separately.
    pub(super) fn native_command_level(host: &LanHost, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        host.native_commands.as_ref()?.levels.get(name).copied()
    }

    /// Resolve the source once at the command boundary, retaining its output origin.
    pub(super) fn native_command_origin(
        &mut self,
        permission: u8,
    ) -> super::commands::R<mod_api::CommandSource> {
        use super::commands::err;
        use mod_api::CommandSource;
        let permission = mod_api::PermissionLevel::from_level(i32::from(permission));
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let origin = if let Some(id) = self.exec.functions.last() {
            CommandSource::Function {
                id: mod_api::Identifier::parse(id).map_err(|_| err("command.failed", &[]))?,
                permission,
            }
        } else if let Some((x, y, z)) = source.block {
            CommandSource::CommandBlock {
                position: [x, y, z],
                level: source.origin_dimension.key(),
                permission,
            }
        } else if let Some(id) = source.cart {
            let entity = self
                .with_source(
                    super::commands::execute::Source {
                        dimension: source.origin_dimension,
                        ..source
                    },
                    |game| game.entity(id),
                )
                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?;
            CommandSource::CommandBlockMinecart { entity, permission }
        } else if source.origin_player {
            let player = self
                .native_acting_player()
                .ok_or_else(|| err("argument.entity.notfound.player", &[]))?;
            CommandSource::Player { player, permission }
        } else {
            CommandSource::Console { permission }
        };
        Ok(origin)
    }

    /// The shared command-node executor, also reached by functions and `execute`.
    pub(super) fn run_native_command(&mut self, context: Context) -> super::commands::R<i32> {
        use super::commands::err;
        let command = self.resolve_native_command(context)?;
        let origin = self.native_command_origin(self.ui.registries.permission)?;
        let mut ran = false;
        self.native_run_mod_command(&origin, &command, &mut ran);
        if ran {
            Ok(1)
        } else {
            Err(err("command.unknown.command", &[]))
        }
    }

    fn native_command_player_target(&self, target: super::commands::Target) -> Option<PlayerRef> {
        use super::commands::Target;
        match target {
            Target::Player => self.native_acting_player(),
            Target::Guest(conn) => self
                .command_guest(conn)
                .and_then(|guest| self.player(guest.uuid)),
            _ => None,
        }
    }

    pub(super) fn native_command_entity_target(
        &mut self,
        target: super::commands::Target,
    ) -> Option<mod_api::EntityRef> {
        if let Some(player) = self.native_command_player_target(target) {
            return Some(player.entity());
        }
        let id = super::native_entities::target_id(target)?;
        self.in_target_level(target, |game| game.entity(id))
            .flatten()
    }

    /// `EntityArgument.getEntity` / `getEntities` / `getPlayer` / `getPlayers`:
    /// convert validated arguments through the shared source-aware selector resolver.
    fn resolve_native_command(
        &mut self,
        context: Context,
    ) -> Result<mod_api::CommandInvocation, horizon_client::command::CommandError> {
        use super::commands::err;
        use mod_api::{CommandArgument, CommandArgumentValue as Value};
        let mut arguments = Vec::with_capacity(context.args.len());
        for argument in context.args {
            let ty = parsed_type(&self.ui.dispatcher.0, &context.nodes, argument.range);
            let value = match argument.value {
                Arg::Str(text) if matches!(ty, Some(ArgType::Nbt)) => nbt_value(&text)?,
                Arg::Str(value) => Value::String(value),
                Arg::Int(value) => Value::Integer(value),
                Arg::Double(value) => Value::Number(value),
                Arg::Bool(value) => Value::Boolean(value),
                Arg::Entity(selector) => {
                    // The parsed node, not its name, determines cardinality: usages may
                    // repeat an argument name or use different kinds on separate branches.
                    let (single, players) = context
                        .nodes
                        .iter()
                        .find_map(|(node, range)| {
                            if *range != argument.range {
                                return None;
                            }
                            match &self.ui.dispatcher.0.nodes[*node].kind {
                                horizon_client::command::NodeKind::Argument {
                                    ty: ArgType::Entity { single, players },
                                    ..
                                } => Some((*single, *players)),
                                _ => None,
                            }
                        })
                        .expect("parsed entity argument has its dispatcher node");
                    let targets = self.resolve_entity_argument(&selector, players)?;
                    if single && targets.len() > 1 {
                        return Err(err(
                            if players {
                                "argument.player.toomany"
                            } else {
                                "argument.entity.toomany"
                            },
                            &[],
                        ));
                    }
                    if players {
                        if single {
                            Value::Player(
                                self.native_command_player_target(targets[0])
                                    .ok_or_else(|| err("argument.entity.notfound.player", &[]))?,
                            )
                        } else {
                            let selected = targets
                                .into_iter()
                                .map(|target| {
                                    self.native_command_player_target(target)
                                        .ok_or_else(|| err("argument.entity.notfound.player", &[]))
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            Value::Players(selected)
                        }
                    } else if single {
                        Value::Entity(
                            self.native_command_entity_target(targets[0])
                                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?,
                        )
                    } else {
                        let selected: Vec<_> = targets
                            .into_iter()
                            .filter_map(|target| self.native_command_entity_target(target))
                            .collect();
                        if selected.is_empty() {
                            return Err(err("argument.entity.notfound.entity", &[]));
                        }
                        Value::Entities(selected)
                    }
                }
                Arg::Coords(coordinates) => positional_value(
                    ty.as_ref(),
                    &coordinates,
                    self.resolve_coords(&coordinates),
                    self.source_rotation(),
                ),
                Arg::Item(input) => Value::Item(
                    self.command_item_stack(&input, 1)?
                        .try_into_api()
                        .expect("command item validation resolves registered items"),
                ),
                Arg::Resource(id) if matches!(ty, Some(ArgType::Dimension)) => {
                    dimension_value(&id, |dimension| self.level(dimension))?
                }
                Arg::Resource(id) => Value::Id(
                    mod_api::Identifier::parse(&id).map_err(|_| err("command.failed", &[]))?,
                ),
                Arg::Custom { kind, value } => custom_value(&kind, value),
                _ => unreachable!("the native tree contains only registered native argument kinds"),
            };
            arguments.push(CommandArgument::new(argument.name, value));
        }
        Ok(self.native_invocation(context.literals, arguments))
    }

    /// A guest's command for the mods, inside their swap
    /// (`run_guest_command`): true when one ran it.
    pub(super) fn perform_native_command(&mut self, conn: u32, permission: u8, line: &str) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        if Self::native_command_level(host, line).is_none_or(|level| level > permission) {
            return false;
        }
        if host.hub.guest(conn).is_none() {
            return false;
        }
        let previous = std::mem::replace(&mut self.ui.registries.permission, permission);
        self.run_command(line);
        self.ui.registries.permission = previous;
        true
    }
}

impl Game<'_> {
    /// A console line for the mods: true when a mod registered its command
    /// (`native_register_commands`). The console is level 4, so a registered
    /// usage is always allowed; a command no mod takes answers as vanilla's
    /// unknown command.
    pub(super) fn perform_native_console_command(&mut self, line: &str) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        if Self::native_command_level(host, line).is_none() {
            return false;
        }
        self.as_console_in_overworld(|game| {
            let previous = game
                .command_source
                .replace(super::commands::execute::Source {
                    silent: false,
                    ..game.server_source()
                });
            game.run_command(line);
            game.command_source = previous;
        });
        true
    }
}

/// The temporary state of the same persistent guest, not another player or action engine.
/// Swapped back after the ordinary guest action has restored the server's outer state.
struct ScopedPlayerState {
    motion: Option<Box<PlayerMotion>>,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    head_yaw: f32,
    on_ground: bool,
    game_mode: i32,
    previous_game_mode: Option<i32>,
    abilities: PlayerAbilities,
    sneaking: bool,
    sprinting: bool,
    fall_flying: bool,
    inventory: Vec<Option<horizon_items::items::ItemStack>>,
    selected: usize,
    armor: [Option<horizon_items::items::ItemStack>; 4],
    offhand: Option<horizon_items::items::ItemStack>,
    carried: Option<horizon_items::items::ItemStack>,
    equipment: horizon_items::equipment::Equipment,
    effects: horizon_client::effects::PlayerEffects,
    health: f32,
    food: i32,
    saturation: f32,
}
impl ScopedPlayerState {
    fn new(
        resources: &horizon_client::motion::MotionResources,
        items: &horizon_items::items::Items,
        setup: PlayerActionSetup,
    ) -> Result<Self, ActionRefused> {
        // Standing eye height even for secondary use, just as a freshly positioned fake.
        let position = [
            setup.eye[0],
            setup.eye[1] - f64::from(1.62_f32),
            setup.eye[2],
        ];
        let mut motion = PlayerMotion::new(
            resources,
            horizon_core::geometry::Vec3::new(position[0], position[1], position[2]),
        )
        .map_err(|_| ActionRefused::InvalidSetup)?;
        motion.shift_key_down = setup.secondary_use;
        let mut inventory = vec![None; 36];
        inventory[0] = setup
            .stack
            .try_into_engine(items)
            .map_err(ActionRefused::InvalidStack)?;
        Ok(Self {
            motion: Some(Box::new(motion)),
            position,
            yaw: setup.yaw,
            pitch: setup.pitch,
            head_yaw: setup.yaw,
            on_ground: false,
            game_mode: 0,
            previous_game_mode: None,
            abilities: PlayerAbilities::default(),
            sneaking: setup.secondary_use,
            sprinting: false,
            fall_flying: false,
            inventory,
            selected: 0,
            armor: Default::default(),
            offhand: None,
            carried: None,
            equipment: Default::default(),
            effects: Default::default(),
            health: 20.0,
            food: 20,
            saturation: 5.0,
        })
    }

    fn swap(&mut self, guest: &mut Guest) {
        std::mem::swap(&mut self.motion, &mut guest.motion);
        std::mem::swap(&mut self.position, &mut guest.position);
        std::mem::swap(&mut self.yaw, &mut guest.yaw);
        std::mem::swap(&mut self.pitch, &mut guest.pitch);
        std::mem::swap(&mut self.head_yaw, &mut guest.head_yaw);
        std::mem::swap(&mut self.on_ground, &mut guest.on_ground);
        std::mem::swap(&mut self.game_mode, &mut guest.game_mode);
        std::mem::swap(&mut self.previous_game_mode, &mut guest.previous_game_mode);
        std::mem::swap(&mut self.abilities, &mut guest.abilities);
        std::mem::swap(&mut self.sneaking, &mut guest.sneaking);
        std::mem::swap(&mut self.sprinting, &mut guest.sprinting);
        std::mem::swap(&mut self.fall_flying, &mut guest.fall_flying);
        std::mem::swap(&mut self.inventory, &mut guest.inventory);
        std::mem::swap(&mut self.selected, &mut guest.selected);
        std::mem::swap(&mut self.armor, &mut guest.armor);
        std::mem::swap(&mut self.offhand, &mut guest.offhand);
        std::mem::swap(&mut self.carried, &mut guest.carried);
        std::mem::swap(&mut self.equipment, &mut guest.equipment);
        std::mem::swap(&mut self.effects, &mut guest.effects);
        std::mem::swap(&mut self.health, &mut guest.health);
        std::mem::swap(&mut self.food, &mut guest.food);
        std::mem::swap(&mut self.saturation, &mut guest.saturation);
    }
}

/// A fake player's `perform_action` in progress: set for its length by [`Acting`], whose drop
/// clears it on every exit, an early return or a panic unwinding through a mod's hook, and the
/// effect it stopped, cleared as each action starts.
#[derive(Default)]
pub(super) struct FakeAction {
    acting: AtomicBool,
    /// The first effect stopped since the action started.
    blocked: Mutex<Option<FakeEffect>>,
}
impl FakeAction {
    /// Whether a fake player's action is running.
    pub(super) fn acting(&self) -> bool {
        self.acting.load(Ordering::Relaxed)
    }
    fn block(&self, effect: FakeEffect) {
        // The first effect stopped is the one the action answers.
        self.blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(effect);
    }
}
/// Marks a fake player's action as running until it drops.
struct Acting(Arc<FakeAction>);
impl Acting {
    fn start(action: &Arc<FakeAction>) -> Self {
        *action
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        action.acting.store(true, Ordering::Relaxed);
        Self(Arc::clone(action))
    }
    /// The effect this action stopped, if any.
    fn blocked(&self) -> Option<FakeEffect> {
        *self
            .0
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
impl Drop for Acting {
    fn drop(&mut self) {
        self.0.acting.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::items::ItemStack;

    /// Box bounds come from mod code: one that is not finite reads no orb and never
    /// panics, and an inverted box reads what the ordered one does.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn experience_orbs_refuse_bounds_that_are_not_finite_and_swap_an_inverted_box() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        let at = horizon_core::geometry::Vec3::new(1.0, 70.0, 1.0);
        game.orbs.push(horizon_client::entities::ExperienceOrb {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            id: 40,
            position: at,
            previous: at,
            velocity: horizon_core::geometry::Vec3::ZERO,
            value: 3,
            age: 0,
            on_ground: true,
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            no_gravity: true,
            following: None,
        });
        let dimension = game.dim.id().to_owned();
        let ids = |game: &Game<'_>, min, max| -> Vec<u32> {
            game.experience_orbs(&dimension, min, max)
                .into_iter()
                .map(|orb| orb.id)
                .collect()
        };
        assert_eq!(ids(&game, [0.0, 69.0, 0.0], [2.0, 71.0, 2.0]), [40]);
        for (min, max) in [
            ([f64::NAN, 69.0, 0.0], [2.0, 71.0, 2.0]),
            ([0.0, 69.0, 0.0], [2.0, f64::NAN, 2.0]),
            ([f64::NEG_INFINITY, 69.0, 0.0], [2.0, 71.0, 2.0]),
            ([0.0, 69.0, 0.0], [2.0, 71.0, f64::INFINITY]),
            ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
        ] {
            assert_eq!(ids(&game, min, max), Vec::<u32>::new(), "{min:?} {max:?}");
        }
        assert_eq!(ids(&game, [2.0, 69.0, 0.0], [0.0, 71.0, 2.0]), [40]);
        assert_eq!(ids(&game, [2.0, 71.0, 2.0], [0.0, 69.0, 0.0]), [40]);
    }

    /// A mod's `announce` reaches the dedicated server's event stream and nothing else's.
    /// The integrated server thread keeps the launch configuration (`options.dedicated`) too,
    /// so it cannot be what decides.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn server_events_are_posted_only_on_the_dedicated_role() {
        use crate::game::execution_role::ExecutionRole;
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        for (role, posted) in [
            (ExecutionRole::DedicatedServer, true),
            (ExecutionRole::IntegratedServer, false),
            (ExecutionRole::Client, false),
        ] {
            game.options.execution_role = role;
            assert_eq!(
                game.native_server_event("Alex", 7, "hello"),
                posted,
                "{role:?}"
            );
        }
    }

    fn with_command_sources(test: impl FnOnce(&mut Game<'_>)) {
        use horizon_client::{
            chunk::Chunk, chunk::block_entity::BlockEntity, command_block::CommandBlock,
        };
        use horizon_core::geometry::Vec3;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            game.register_native_commands();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:command_block")
                .unwrap();
            game.world.set(1, 64, 1, state).unwrap();
            game.world.block_entities.insert(
                (1, 64, 1),
                BlockEntity::CommandBlock(Box::new(CommandBlock {
                    command: "execute in minecraft:the_nether run origin".into(),
                    condition_met: true,
                    ..Default::default()
                })),
            );
            let mut cart = super::super::minecarts::Minecart::new(
                99,
                super::super::minecarts::CartKind::CommandBlock,
                Vec3::new(1.0, 64.0, 1.0),
            );
            cart.command.command = "origin".into();
            game.carts.push(cart);
            test(game);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn native_command_feedback_respects_track_output_and_silence() {
        use super::super::commands::execute::Source;
        use horizon_client::chunk::block_entity::BlockEntity;
        with_command_sources(|game| {
            let source = Source {
                block: Some((1, 64, 1)),
                silent: false,
                ..game.server_source()
            };
            for track in [true, false] {
                if let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get_mut(&(1, 64, 1))
                {
                    block.track_output = track;
                    block.last_output = None;
                }
                game.with_source(source, |game| game.native_feedback(Text::literal("answer")));
                let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get(&(1, 64, 1))
                else {
                    panic!("block missing")
                };
                assert_eq!(block.last_output.is_some(), track);
            }
            game.chat_capture = Some(Vec::new());
            game.with_source(
                Source {
                    silent: true,
                    block: None,
                    ..source
                },
                |game| game.native_feedback(Text::literal("silent")),
            );
            assert!(game.chat_capture.take().unwrap().is_empty());
            assert!(game.command_source.is_none());
            game.as_guest(12, |game| {
                let source = game.server_source();
                game.with_source(source, |game| {
                    assert!(matches!(
                        game.native_command_origin(4),
                        Ok(mod_api::CommandSource::Console { .. })
                    ));
                });
            })
            .unwrap();
            game.exec.functions.push("test:outer".into());
            assert!(game.native_run_command("list"));
            assert_eq!(game.exec.functions, ["test:outer"]);
        });
    }

    type Heard = Vec<(&'static str, u32, u128, &'static str, [i32; 2])>;
    /// Logs each of the watch trio: (hook, player id, UUID, level, chunk).
    struct Watcher(std::sync::Arc<std::sync::Mutex<Heard>>);
    impl Watcher {
        fn log(&self, hook: &'static str, player: PlayerRef, chunk: mod_api::ChunkRef) {
            self.0.lock().unwrap().push((
                hook,
                player.entity().id(),
                player.uuid(),
                chunk.level().dimension_id(),
                chunk.pos(),
            ));
        }
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watcher {
        fn chunk_watch(&self, _: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkWatchEvent) {
            self.log("watch", e.player, e.chunk);
        }
        fn chunk_sent(&self, _: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkSentEvent) {
            self.log("sent", e.player, e.chunk);
        }
        fn chunk_unwatch(
            &self,
            _: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::ChunkUnwatchEvent,
        ) {
            self.log("unwatch", e.player, e.chunk);
        }
    }

    /// `ChunkWatchEvent`: the hub's records reach the mods in order, each
    /// with the player (entity id and UUID, even after they left) and the
    /// chunk in its level; a level the server doesn't hold is skipped.
    #[test]
    #[ignore = "needs prepared resources"]
    fn the_hubs_chunk_watches_reach_the_mods_in_order() {
        use horizon_client::lan::chunk_watch::{
            ChunkWatch, ChunkWatchKind, ChunkWatchLog, LevelWatches,
        };
        let heard = std::sync::Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "watch",
            Box::new(Watcher(std::sync::Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                super::super::native_item_places::tests::open_to_lan(game);
                let here = game.dim.id();
                let watch = |kind, chunk| ChunkWatch {
                    kind,
                    uuid: 127,
                    entity_id: 42,
                    chunk,
                };
                let run = |dimension: &str, watches| LevelWatches {
                    dimension: dimension.into(),
                    watches,
                };
                game.lan.as_mut().unwrap().hub.chunk_watches = ChunkWatchLog::new(vec![
                    run(
                        here,
                        vec![
                            watch(ChunkWatchKind::Watch, [1, 2]),
                            watch(ChunkWatchKind::Sent, [1, 2]),
                        ],
                    ),
                    run(
                        "minecraft:nowhere",
                        vec![watch(ChunkWatchKind::Unwatch, [0, 0])],
                    ),
                    run(here, vec![watch(ChunkWatchKind::Unwatch, [1, 2])]),
                ]);
                game.run_chunk_watches();
                assert!(game.lan.as_ref().unwrap().hub.chunk_watches.is_empty());
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("watch", 42, 127, here, [1, 2]),
                        ("sent", 42, 127, here, [1, 2]),
                        ("unwatch", 42, 127, here, [1, 2]),
                    ]
                );
            });
        });
    }

    fn player_guest() -> Guest {
        use horizon_client::lan::{Joined, PlayerSender};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        Guest::new(
            Joined {
                conn: 1,
                uuid: 123,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.0; 3],
            10,
        )
    }

    #[test]
    fn player_reads_use_guest_mode_and_abilities() {
        let mut guest = player_guest();
        guest.abilities.flying_speed = 0.08;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), 0);
        assert_eq!(
            read.food(),
            None,
            "food is unavailable before body creation"
        );
        assert_eq!(
            read.abilities(),
            Some(mod_api::PlayerAbilities::from(
                mod_api::__private::built::PlayerAbilities {
                    invulnerable: false,
                    flying: false,
                    may_fly: false,
                    instabuild: false,
                    may_build: true,
                    fly_speed: 0.08,
                    walk_speed: 0.1,
                }
            ))
        );
        guest.game_mode = 1;
        guest.abilities.update_for_mode(1);
        guest.abilities.flying = true;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), 1);
        assert_eq!(
            read.abilities(),
            Some(mod_api::PlayerAbilities::from(
                mod_api::__private::built::PlayerAbilities {
                    invulnerable: true,
                    flying: true,
                    may_fly: true,
                    instabuild: true,
                    may_build: true,
                    fly_speed: 0.08,
                    walk_speed: 0.1,
                }
            ))
        );
        guest.game_mode = 3;
        guest.abilities.update_for_mode(3);
        let abilities = PlayerRead::Guest(&guest).abilities().unwrap();
        assert!(!abilities.may_build && !abilities.instabuild && abilities.may_fly);
    }

    fn tree(commands: &[mod_api::Command]) -> (Value, HashMap<String, u8>, Vec<String>) {
        command_tree(commands, |name| name == "give", &HashMap::new())
    }

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    #[test]
    fn orbs_are_found_by_their_half_block_box() {
        // A block's cell, as a mod asks for a broken block's drops.
        let (min, max) = ([10.0, 64.0, -3.0], [11.0, 65.0, -2.0]);
        assert!(orb_meets([10.5, 64.2, -2.5], min, max), "inside");
        assert!(
            orb_meets([11.2, 64.0, -2.5], min, max),
            "its box reaches in"
        );
        assert!(
            orb_meets([10.5, 63.6, -2.5], min, max),
            "standing just below"
        );
        assert!(!orb_meets([11.25, 64.0, -2.5], min, max), "a face touching");
        assert!(!orb_meets([10.5, 63.5, -2.5], min, max), "its top touching");
        assert!(!orb_meets([10.5, 65.0, -2.5], min, max), "standing on top");
    }

    #[test]
    fn whole_stacks_are_given_onto_equal_stacks_only() {
        use super::super::inventory_add;
        let mut inv = vec![stack("minecraft:dirt", 64); 36];
        inv[2] = stack("minecraft:iron_sword", 1);
        inv[2].as_mut().unwrap().damage = 5;
        inv[5] = stack("minecraft:arrow", 60);
        inv[7] = None;
        inv[20] = None;
        let mut off = stack("minecraft:arrow", 62);

        // A damaged sword never joins another stack: the first free slot.
        let mut sword = stack("minecraft:iron_sword", 1).unwrap();
        sword.damage = 40;
        assert!(!inventory_add(&mut inv, &mut off, 5, 1, &mut sword));
        assert_eq!(inv[7].as_ref().unwrap().damage, 40, "its damage is kept");
        assert_eq!(inv[2].as_ref().unwrap().damage, 5);

        // Arrows: the selected slot, then the off hand, then a free slot,
        // and what is left over comes back.
        let mut arrows = stack("minecraft:arrow", 80).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 5, 64, &mut arrows));
        assert_eq!(inv[5].as_ref().unwrap().count, 64);
        assert_eq!(off.as_ref().unwrap().count, 64);
        assert_eq!(inv[20].as_ref().unwrap().count, 64);
        assert_eq!(arrows.count, 80 - 4 - 2 - 64);

        // A full inventory with an empty off hand takes nothing.
        let mut off = None;
        let mut apple = stack("minecraft:apple", 3).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 0, 64, &mut apple));
        assert!(off.is_none(), "never into an empty off hand (getFreeSlot)");
        assert_eq!(apple.count, 3);
    }

    #[test]
    fn usages_merge_into_one_tree_with_runnable_nodes() {
        let (t, levels, problems) = tree(&[
            mod_api::Command::new("balance [player:player]"),
            mod_api::Command::new("pay <player:player> <amount:integer>"),
            mod_api::Command::op("eco give <player:player> <amount:integer>", 3),
            mod_api::Command::op("eco take <player:player> <amount:integer>", 3),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let c = &t["children"];
        assert_eq!(c["balance"]["executable"], true);
        assert_eq!(c["balance"]["children"]["player"]["executable"], true);
        assert_eq!(
            c["balance"]["children"]["player"]["parser"],
            "minecraft:entity"
        );
        assert_eq!(c["pay"]["executable"], false);
        assert_eq!(c["pay"]["children"]["player"]["executable"], false);
        let amount = &c["pay"]["children"]["player"]["children"]["amount"];
        assert_eq!(amount["executable"], true);
        assert_eq!(amount["parser"], "brigadier:integer");
        assert!(amount.get("children").is_none());
        assert_eq!(c["eco"]["permissions"]["permission"]["level"], "admins");
        assert!(c["eco"]["children"]["give"].is_object());
        assert!(c["eco"]["children"]["take"].is_object());
        assert!(c["pay"].get("permissions").is_none());
        assert_eq!(levels["eco"], 3);
        assert_eq!(levels["pay"], 0);
    }

    #[test]
    fn a_usage_with_another_string_kind_is_left_out_in_either_order() {
        for (first, second, line, other) in [
            (
                "foo <value:word>",
                "foo <value:text>",
                "foo hello",
                "foo hello world",
            ),
            (
                "foo <value:text>",
                "foo <value:word>",
                "foo hello world",
                "foo hello",
            ),
        ] {
            let (tree, _, problems) =
                tree(&[mod_api::Command::new(first), mod_api::Command::new(second)]);
            assert_eq!(problems.len(), 1, "{first} then {second}: {problems:?}");
            assert!(problems[0].starts_with(&format!("`{second}`")));
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            let registries = horizon_client::command::Registries::default();
            assert!(dispatcher.parse(line, &registries).executable().is_ok());
            // The kept kind decides: `word` stops at the space, `text` takes it all.
            let kept_greedy = first.contains(":text");
            assert_eq!(
                dispatcher.parse(other, &registries).executable().is_ok(),
                kept_greedy,
                "{first} then {second}: {other}"
            );
        }
    }

    const POSITIONED: &str = "kinds <at:vec3> <by:vec3_uncentered> <column:column_pos> \
         <look:rotation> <level:dimension> <tag:nbt>";

    #[test]
    fn position_rotation_dimension_and_nbt_kinds_are_vanillas_parsers() {
        let (tree, _, problems) = tree(&[mod_api::Command::new(POSITIONED)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut node = &tree["children"]["kinds"];
        for (name, parser) in [
            ("at", "minecraft:vec3"),
            ("by", "minecraft:vec3"),
            ("column", "minecraft:column_pos"),
            ("look", "minecraft:rotation"),
            ("level", "minecraft:dimension"),
            ("tag", "minecraft:nbt_compound_tag"),
        ] {
            node = &node["children"][name];
            assert_eq!(node["parser"], parser, "{name}");
            assert_eq!(node["uncentered"] == true, name == "by", "{name}");
        }
    }

    #[test]
    fn vec3_and_vec3_uncentered_do_not_share_a_node() {
        let (_, _, problems) = tree(&[
            mod_api::Command::new("go <to:vec3>"),
            mod_api::Command::new("go <to:vec3_uncentered>"),
        ]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("another kind"), "{problems:?}");
    }

    #[test]
    fn positions_and_rotations_resolve_against_the_source() {
        let (tree, _, problems) = tree(&[mod_api::Command::new(POSITIONED)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let registries = horizon_client::command::Registries::default();
        let (origin, rotation) = ([10.0, 70.0, -2.0], [45.0, 30.0]);
        let value = |line: &str, name: &str, kind: &str| {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed.executable().unwrap();
            let Some(Arg::Coords(coordinates)) = context.get(name) else {
                panic!("{name} is a position")
            };
            let range = context.args.iter().find(|a| a.name == name).unwrap().range;
            let ty = parsed_type(&dispatcher, &context.nodes, range);
            assert_eq!(
                format!("{ty:?}"),
                format!("{:?}", Some(parser(kind).unwrap().2)),
                "{name}: the type comes from the parsed node"
            );
            positional_value(
                ty.as_ref(),
                coordinates,
                coordinates.position(origin, rotation),
                rotation,
            )
        };
        let relative = "kinds ~1 ~ ~-1 ~1 ~ ~-1 ~3 ~-4 ~10 ~-5 minecraft:the_nether {}";
        assert_eq!(
            value(relative, "at", "vec3"),
            mod_api::CommandArgumentValue::Vec3([11.0, 70.0, -3.0])
        );
        assert_eq!(
            value(relative, "column", "column_pos"),
            mod_api::CommandArgumentValue::ColumnPos([13, -6])
        );
        assert_eq!(
            value(relative, "look", "rotation"),
            mod_api::CommandArgumentValue::Rotation([55.0, 25.0])
        );
        // A whole `x` and `z` is centred by `vec3` and kept by `vec3_uncentered`.
        let absolute = "kinds 5 64 -7 5 64 -7 0 0 0 0 minecraft:overworld {}";
        assert_eq!(
            value(absolute, "at", "vec3"),
            mod_api::CommandArgumentValue::Vec3([5.5, 64.0, -6.5])
        );
        assert_eq!(
            value(absolute, "by", "vec3_uncentered"),
            mod_api::CommandArgumentValue::Vec3([5.0, 64.0, -7.0])
        );
    }

    #[test]
    fn a_dimension_the_world_lacks_is_refused_and_an_nbt_argument_is_a_compound() {
        let refusal = dimension_value("minecraft:nowhere", |_| None).unwrap_err();
        assert_eq!(refusal.key, "argument.dimension.invalid");
        let mut expected = horizon_core::nbt::Compound::new();
        expected.insert("answer".into(), horizon_core::nbt::Tag::Byte(42));
        assert_eq!(
            nbt_value("{answer:42b}").unwrap(),
            mod_api::CommandArgumentValue::Nbt(expected)
        );
    }

    #[test]
    fn every_supported_kind_has_a_parser() {
        assert!(
            argument_kinds::KINDS
                .into_iter()
                .all(|kind| parser(kind).is_some())
        );
    }

    /// A test mod's argument type: named machines, one word, completed by prefix.
    struct MachineArgument;

    impl mod_api::CommandArgumentType for MachineArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            match input {
                "press" | "furnace" => Ok(input.to_owned()),
                _ => Err(mod_api::CommandArgumentError::new(
                    mod_api::Text::translate(
                        "test.commands.machine.unknown",
                        vec![mod_api::Text::literal(input)],
                    ),
                )),
            }
        }

        fn suggest(&self, prefix: &str) -> Vec<mod_api::CommandSuggestion> {
            ["press", "furnace"]
                .into_iter()
                .filter(|machine| machine.starts_with(prefix))
                .map(mod_api::CommandSuggestion::new)
                .collect()
        }
    }

    struct HostileArgument;

    impl mod_api::CommandArgumentType for HostileArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }
        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            if input == "panic" {
                panic!("parser")
            }
            Ok(input.into())
        }
        fn suggest(&self, prefix: &str) -> Vec<mod_api::CommandSuggestion> {
            match prefix {
                "panic" => panic!("completer"),
                "text" => vec![mod_api::CommandSuggestion::new("a".repeat(32768))],
                "tip" => vec![mod_api::CommandSuggestion::with_tooltip(
                    "ok",
                    "a".repeat(65536),
                )],
                "unicode" => vec![mod_api::CommandSuggestion::with_tooltip(
                    "ok",
                    "\0".repeat(32768),
                )],
                _ => vec![mod_api::CommandSuggestion::with_tooltip(
                    "a".repeat(32767),
                    "a".repeat(65535),
                )],
            }
        }
    }

    #[test]
    fn hostile_argument_callbacks_are_refused_without_losing_the_dispatcher() {
        let arguments: CommandArguments = HashMap::from([(
            "test:hostile".into(),
            Arc::new(HostileArgument) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = super::command_tree(
            &[mod_api::Command::new("probe <value:test:hostile>")],
            |_| false,
            &arguments,
        );
        assert!(problems.is_empty());
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = horizon_client::command::Registries::default();
        for prefix in ["panic", "text", "tip", "unicode"] {
            let input = format!("probe {prefix}");
            let parsed = dispatcher.parse(&input, &registries);
            assert_eq!(parsed.executable().is_err(), prefix == "panic");
            let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
            assert!(suggestions.is_empty(), "{prefix}");
        }
        let input = "probe valid";
        let parsed = dispatcher.parse(input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text.len(), 32767);
    }

    #[test]
    fn panicking_argument_kind_is_left_out_of_the_tree() {
        struct PanicKind;
        impl mod_api::CommandArgumentType for PanicKind {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                panic!("kind")
            }
            fn parse(&self, _: &str) -> Result<String, mod_api::CommandArgumentError> {
                unreachable!()
            }
            fn suggest(&self, _: &str) -> Vec<mod_api::CommandSuggestion> {
                unreachable!()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:panic".into(),
            Arc::new(PanicKind) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (_, _, problems) = super::command_tree(
            &[mod_api::Command::new("probe <value:test:panic>")],
            |_| false,
            &arguments,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("panicked"));
    }

    struct Machines;

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Machines {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_command_argument("machine", MachineArgument);
        }
    }

    #[test]
    fn a_registered_argument_type_parses_and_completes_its_values() {
        mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
            "test",
            Box::new(Machines),
        )])
        .unwrap();
        let arguments = command_arguments();
        assert!(arguments.contains_key("test:machine"));

        let (tree, _, problems) = super::command_tree(
            &[mod_api::Command::new("machine use <machine:test:machine>")],
            |_| false,
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let machine = &tree["children"]["machine"]["children"]["use"]["children"]["machine"];
        assert_eq!(machine["custom_argument"], "test:machine");
        assert_eq!(machine["suggestions"], "minecraft:ask_server");
        assert_eq!(machine["properties"], json!({"type": "word"}));

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = horizon_client::command::Registries::default();
        let parsed = dispatcher.parse("machine use press", &registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("machine"),
            Some(&Arg::Custom {
                kind: "test:machine".into(),
                value: "press".into(),
            })
        );
        assert!(
            dispatcher
                .parse("machine use banana", &registries)
                .executable()
                .is_err(),
            "the mod's parser rejects what it does not know"
        );

        let typed = "machine use pr";
        let parse = dispatcher.parse(typed, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["press"]
        );
    }

    #[test]
    fn resource_and_mod_id_kinds_accept_only_known_ids() {
        let mut arguments = command_arguments();
        // The test process loads no package: stand in for one the way `command_arguments` does.
        arguments.insert(
            argument_kinds::MOD_ID.into(),
            Arc::new(mod_api::ChoiceArgument::new(["demo"])),
        );
        let (tree, _, problems) = super::command_tree(
            &[
                mod_api::Command::new("spawnof <kind:resource:minecraft:entity_type>"),
                mod_api::Command::new("about <id:mod_id>"),
                mod_api::Command::new("bad <kind:resource:NotOk>"),
                mod_api::Command::new("worse <kind:resource:a:b:c>"),
                mod_api::Command::new("worst <kind:resource:a/b:c>"),
            ],
            |_| false,
            &arguments,
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(
            problems
                .iter()
                .all(|p| p.contains("is not a `namespace:path` resource location")),
            "{problems:?}"
        );
        for refused in ["bad", "worse", "worst"] {
            assert!(tree["children"].get(refused).is_none(), "{refused}");
        }
        let kind = &tree["children"]["spawnof"]["children"]["kind"];
        assert_eq!(kind["parser"], "minecraft:resource_key");
        assert_eq!(
            kind["properties"],
            json!({"registry": "minecraft:entity_type"})
        );
        assert_eq!(
            tree["children"]["about"]["children"]["id"]["custom_argument"],
            argument_kinds::MOD_ID
        );

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = horizon_client::command::Registries {
            entity_types: vec!["minecraft:zombie".into()],
            ..Default::default()
        };
        let parsed = dispatcher.parse("spawnof minecraft:zombie", &registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("kind"),
            Some(&Arg::Resource("minecraft:zombie".into()))
        );
        for refused in ["spawnof minecraft:nope", "about no_such_mod"] {
            assert!(
                dispatcher.parse(refused, &registries).executable().is_err(),
                "{refused}"
            );
        }
        let parsed = dispatcher.parse("about demo", &registries);
        let (context, _) = parsed.executable().unwrap();
        let Some(Arg::Custom { kind, value }) = context.get("id").cloned() else {
            panic!("a mod id parses as the built-in custom kind");
        };
        assert_eq!(
            custom_value(&kind, value),
            mod_api::CommandArgumentValue::String("demo".into()),
            "a loaded mod id reaches the mod as its plain string"
        );
    }

    #[test]
    fn native_usages_parse_with_their_permissions_in_the_server_dispatcher() {
        let (tree, _, problems) = tree(&[
            mod_api::Command::new("warp [name:word]"),
            mod_api::Command::op("warp set <amount:integer>", 2),
            mod_api::Command::new("toggle <enabled:boolean>"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let mut registries = horizon_client::command::Registries::default();
        assert!(dispatcher.parse("warp", &registries).executable().is_ok());
        assert!(
            dispatcher
                .parse("warp home", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("warp set 7", &registries)
                .executable()
                .is_err()
        );
        registries.permission = 2;
        assert!(
            dispatcher
                .parse("warp set 7", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("warp set nope", &registries)
                .executable()
                .is_err()
        );
        assert!(
            dispatcher
                .parse("warp set", &registries)
                .executable()
                .is_err()
        );
        assert!(
            dispatcher
                .parse("toggle true", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("toggle maybe", &registries)
                .executable()
                .is_err()
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn command_refresh_rebuilds_the_typed_dispatcher_only_on_request() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        struct ChangingCommands {
            changed: Arc<AtomicBool>,
            registrations: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ChangingCommands {
            fn register_commands(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RegisterCommandsEvent<'_>,
            ) {
                self.registrations.fetch_add(1, Ordering::Relaxed);
                event.commands.push(mod_api::Command::new(
                    if self.changed.load(Ordering::Relaxed) {
                        "after <n:integer>"
                    } else {
                        "before"
                    },
                ));
            }
        }
        let changed = Arc::new(AtomicBool::new(false));
        let registrations = Arc::new(AtomicUsize::new(0));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "refresh",
                Box::new(ChangingCommands {
                    changed: changed.clone(),
                    registrations: registrations.clone(),
                }),
            )]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    game.build_registries();
                    game.register_native_commands();
                    assert_eq!(registrations.load(Ordering::Relaxed), 1);
                    changed.store(true, Ordering::Relaxed);
                    game.register_native_commands();
                    assert_eq!(registrations.load(Ordering::Relaxed), 1);
                    let host = game.lan.as_ref().unwrap();
                    assert!(
                        host.native_commands
                            .as_ref()
                            .unwrap()
                            .levels
                            .contains_key("before")
                    );
                    mod_api::Commands::refresh_commands(game);
                    mod_api::Commands::refresh_commands(game);
                    assert_eq!(
                        registrations.load(Ordering::Relaxed),
                        1,
                        "requests are deferred"
                    );
                    game.register_native_commands();
                    assert!(!game.native_commands_refresh);
                    assert_eq!(
                        registrations.load(Ordering::Relaxed),
                        2,
                        "requests coalesce"
                    );
                    let commands = game.lan.as_ref().unwrap().native_commands.as_ref().unwrap();
                    assert!(!commands.levels.contains_key("before"));
                    assert!(commands.levels.contains_key("after"));
                    let dispatcher = &game.ui.dispatcher.0;
                    assert!(
                        dispatcher
                            .parse("after 7", &game.ui.registries)
                            .executable()
                            .is_ok()
                    );
                    assert!(
                        dispatcher
                            .parse("after nope", &game.ui.registries)
                            .executable()
                            .is_err()
                    );
                    assert!(
                        dispatcher
                            .parse("before", &game.ui.registries)
                            .executable()
                            .is_err()
                    );
                    game.register_native_commands();
                    assert_eq!(registrations.load(Ordering::Relaxed), 2);
                    mod_api::Commands::refresh_commands(game);
                    game.register_native_commands();
                    assert_eq!(registrations.load(Ordering::Relaxed), 3);
                    assert!(
                        Game::native_command_level(game.lan.as_ref().unwrap(), "after").is_some()
                    );
                })
            },
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn native_typed_commands_reach_listeners_after_parse_and_source_resolution() {
        use mod_api::{CommandArgumentValue as Value, CommandInvocation};
        struct Commands(Arc<Mutex<Vec<(bool, CommandInvocation)>>>);
        impl Commands {
            fn record(&self, console: bool, command: &CommandInvocation) {
                if command.literals == ["typed", "give"] {
                    let Some(Value::Integer(amount)) = command.argument("amount") else {
                        panic!("listener must receive an integer")
                    };
                    let Some(Value::Player(player)) = command.argument("who") else {
                        panic!("listener must receive a resolved player")
                    };
                    assert_ne!(*amount, 0);
                    assert_ne!(player.uuid(), 0);
                }
                self.0.lock().unwrap().push((console, command.clone()));
            }
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Commands {
            fn register_commands(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RegisterCommandsEvent<'_>,
            ) {
                event.commands.extend([
                    mod_api::Command::new("typed give <who:player> <amount:integer> [message:text]"),
                    mod_api::Command::new("typed data <word:word> <number:number> <enabled:boolean> <item:item> <pos:block_pos>"),
                    mod_api::Command::new("typed opt [value:integer]"),
                    mod_api::Command::new("typed repeat <value:player> <value:integer>"),
                    mod_api::Command::op("protected <who:player>", 4),
                ]);
            }
            fn run_mod_command(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RunModCommandEvent<'_>,
            ) -> mod_api::Flow {
                self.record(
                    matches!(event.source, mod_api::CommandSource::Console { .. }),
                    event.command,
                );
                mod_api::Flow::Cancel
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "typed_commands",
                Box::new(Commands(heard.clone())),
            )]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    game.build_registries();
                    game.register_native_commands();
                    let other = game.player(13).unwrap();
                    let acting = game.player(12).unwrap();
                    game.as_guest(12, |game| {
                        for line in [
                            "typed give guest13 nope",
                            "typed give guest13",
                            "typed give missing 7",
                            "typed give @a 7",
                            "typed data word 1 maybe stone ~ ~ ~",
                            "typed data word NaN true stone ~ ~ ~",
                            "typed data word 1 true does_not_exist ~ ~ ~",
                            "typed opt 1 extra",
                        ] {
                            assert!(game.perform_native_command(12, 4, line), "{line}");
                        }
                        let random = game.world.random.state();
                        assert!(!game.perform_native_command(12, 0, "protected @r"));
                        assert_eq!(game.world.random.state(), random, "denied selectors must not resolve");
                        assert!(heard.lock().unwrap().is_empty(), "failed input must not reach listeners");
                        for line in [
                            "typed give guest13 37 hello typed world",
                            "typed give @s -4",
                            "typed opt",
                            "typed opt 9",
                            "typed data word -1.25 true stone[minecraft:custom_name='\"Rock\"'] ~1 ~2 ~-3",
                            "typed data word 2 false dirt ^0 ^0 ^0",
                        ] {
                            assert!(game.perform_native_command(12, 4, line), "{line}");
                        }
                    }).unwrap();
                    game.spawn = Some(horizon_core::geometry::Vec3::new(10.5, 70.0, -2.5));
                    game.yaw = 45.0;
                    game.pitch = 30.0;
                    for (line, key) in [
                        ("typed give guest13 nope", "parsing.int.expected"),
                        ("typed give missing 1", "argument.entity.notfound.player"),
                    ] {
                        game.chat_capture = Some(Vec::new());
                        assert!(game.perform_native_console_command(line));
                        let output = game.chat_capture.take().unwrap();
                        assert_eq!(
                            output[0].plain(),
                            Text::translate(key, &[]).plain(),
                            "{line}"
                        );
                    }
                    assert!(game.perform_native_console_command("typed give @s 1"));
                    assert_eq!(
                        heard.lock().unwrap().len(),
                        6,
                        "console has no source player"
                    );
                    assert!(game.perform_native_console_command("typed give guest13 11"));
                    assert!(
                        game.perform_native_console_command(
                            "typed data console 3 true dirt ~1 ~2 ~3"
                        )
                    );
                    assert!(game.perform_native_console_command(
                        "typed data console 3 true dirt ^0.5 ^0.5 ^0.5"
                    ));
                    assert!(game.perform_native_console_command("typed repeat guest13 3"));
                    assert!(game.command_source.is_none(), "console source restored");
                    game.as_guest(12, |game| {
                        assert!(game.native_run_command("execute as guest13 run typed opt 13"))
                    })
                    .unwrap();
                    let heard = heard.lock().unwrap();
                    assert_eq!(heard.len(), 11);
                    let first = &heard[0].1;
                    assert_eq!(first.literals, ["typed", "give"]);
                    assert_eq!(first.argument("who"), Some(&Value::Player(other)));
                    assert_eq!(first.argument("amount"), Some(&Value::Integer(37)));
                    assert_eq!(
                        first.argument("message"),
                        Some(&Value::String("hello typed world".into()))
                    );
                    assert_eq!(heard[1].1.argument("who"), Some(&Value::Player(acting)));
                    assert_eq!(heard[1].1.argument("amount"), Some(&Value::Integer(-4)));
                    assert!(heard[1].1.argument("message").is_none());
                    assert!(heard[2].1.arguments.is_empty());
                    assert_eq!(heard[3].1.argument("value"), Some(&Value::Integer(9)));
                    let data = &heard[4].1;
                    assert_eq!(data.argument("word"), Some(&Value::String("word".into())));
                    assert_eq!(data.argument("number"), Some(&Value::Number(-1.25)));
                    assert_eq!(data.argument("enabled"), Some(&Value::Boolean(true)));
                    assert_eq!(data.argument("pos"), Some(&Value::BlockPos([4, 66, 2])));
                    let Some(Value::Item(item)) = data.argument("item") else {
                        panic!("typed item")
                    };
                    assert_eq!(
                        mod_api::__private::item_key_id(item.item()),
                        Some("minecraft:stone")
                    );
                    assert_eq!(item.count(), 1);
                    assert!(item.components().is_some(), "command components retained");
                    assert_eq!(heard[5].1.argument("enabled"), Some(&Value::Boolean(false)));
                    assert_eq!(
                        heard[5].1.argument("pos"),
                        Some(&Value::BlockPos([3, 64, 5]))
                    );
                    assert!(heard[..6].iter().all(|(console, _)| !console));
                    assert!(heard[6..].iter().all(|(console, _)| *console));
                    assert_eq!(heard[6].1.argument("who"), Some(&Value::Player(other)));
                    assert_eq!(heard[6].1.argument("amount"), Some(&Value::Integer(11)));
                    assert_eq!(
                        heard[7].1.argument("pos"),
                        Some(&Value::BlockPos([11, 72, 0]))
                    );
                    assert_eq!(
                        heard[8].1.argument("pos"),
                        Some(&Value::BlockPos([10, 70, -3]))
                    );
                    assert_eq!(heard[9].1.arguments[0].value, Value::Player(other));
                    assert_eq!(heard[9].1.argument("value"), Some(&Value::Integer(3)));
                });
            },
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn native_commands_keep_function_block_and_cart_sources() {
        use horizon_client::{
            chunk::block_entity::BlockEntity,
            datapack::{Function, FunctionLine},
        };
        use horizon_core::position::BlockPos;
        use mod_api::{CommandSource, PermissionLevel as Permission};
        struct Commands(Arc<Mutex<Vec<CommandSource>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Commands {
            fn register_commands(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RegisterCommandsEvent<'_>,
            ) {
                event.commands.extend([
                    mod_api::Command::new("origin"),
                    mod_api::Command::op("denied", 4),
                    mod_api::Command::new("unhandled"),
                ]);
            }
            fn run_mod_command(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RunModCommandEvent<'_>,
            ) -> mod_api::Flow {
                if event.command.literals == ["unhandled"] {
                    return mod_api::Flow::Continue;
                }
                self.0.lock().unwrap().push(event.source.clone());
                host.commands()
                    .feedback(mod_api::Text::literal("source answer"))
                    .unwrap();
                mod_api::Flow::Cancel
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "origins",
                Box::new(Commands(heard.clone())),
            )]
            .into(),
            || {
                with_command_sources(|game| {
                    game.command_block_tick(BlockPos::new(1, 64, 1));
                    let Some(BlockEntity::CommandBlock(block)) =
                        game.world.block_entities.get(&(1, 64, 1))
                    else {
                        panic!("block missing")
                    };
                    assert_eq!(block.success_count, 1);
                    let source = heard.lock().unwrap()[0].clone();
                    assert!(
                        matches!(source, CommandSource::CommandBlock { position: [1, 64, 1], level, .. } if level == game.dim.key())
                    );
                    assert_eq!(source.permission(), Permission::Gamemasters);
                    assert!(game.perform_cart_command(99));
                    assert_eq!(game.carts[0].command.success_count, 1);
                    assert!(game.carts[0].command.last_output.is_some());
                    assert!(
                        matches!(heard.lock().unwrap().last().unwrap(), CommandSource::CommandBlockMinecart { entity, .. } if entity.id() == 99)
                    );
                    game.chat_capture = Some(Vec::new());
                    let mut function = Function {
                        id: "test:inner".into(),
                        lines: vec![
                            FunctionLine::Command("execute positioned 2 64 3 run origin".into()),
                            FunctionLine::Command("denied".into()),
                        ],
                        parameters: vec![],
                    };
                    game.ensure_datapacks();
                    game.fns
                        .store
                        .functions
                        .insert(function.id.clone(), function.clone());
                    function.id = "test:outer".into();
                    function.lines = vec![FunctionLine::Command("function test:inner".into())];
                    let before = heard.lock().unwrap().len();
                    game.run_function(&function, None, game.server_source())
                        .unwrap();
                    assert_eq!(heard.lock().unwrap().len(), before + 1);
                    assert!(
                        matches!(heard.lock().unwrap().last().unwrap(), CommandSource::Function { id, permission } if id.to_string() == "test:inner" && *permission == Permission::Gamemasters)
                    );
                    assert!(
                        game.chat_capture.take().unwrap().is_empty(),
                        "functions suppress both success and failure"
                    );
                    assert!(game.exec.functions.is_empty());
                    game.ui.registries.permission = 4;
                    assert!(!game.run_command("unhandled"));
                    assert!(game.run_command(
                        "execute store result storage test:result value int 1 run origin"
                    ));
                    assert!(
                        game.run_command(
                            "execute if data storage test:result {value:1} run origin"
                        )
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn native_unauthorized_commands_never_reach_the_listener() {
        struct Commands(Arc<Mutex<Vec<mod_api::CommandInvocation>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Commands {
            fn register_commands(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RegisterCommandsEvent<'_>,
            ) {
                event.commands.extend([
                    mod_api::Command::op("foo [value:integer]", 4),
                    mod_api::Command::new("foo bar"),
                    mod_api::Command::op("choice <a:integer>", 4),
                    mod_api::Command::new("choice <a:integer> bar"),
                    mod_api::Command::new("choice <b:word>"),
                ]);
            }
            fn run_mod_command(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RunModCommandEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push(event.command.clone());
                mod_api::Flow::Cancel
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "commands",
            Box::new(Commands(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                super::super::native_item_places::tests::open_to_lan(game);
                game.register_native_commands();
                game.lan.as_mut().unwrap().hub.guests.push(player_guest());
                for line in ["foo", "foo 7", "foo nope"] {
                    assert!(game.perform_native_command(1, 0, line));
                }
                assert!(
                    heard.lock().unwrap().is_empty(),
                    "denied usages must not call a mod"
                );
                assert!(game.perform_native_command(1, 0, "foo bar"));
                assert!(game.perform_native_command(1, 4, "foo"));
                assert!(game.perform_native_command(1, 4, "foo 7"));
                for line in ["choice 7", "choice 7 bar", "choice text"] {
                    assert!(game.perform_native_command(1, 0, line));
                }
                let heard = heard.lock().unwrap();
                assert_eq!(heard.len(), 6);
                assert_eq!(heard[0].literals, ["foo", "bar"]);
                assert_eq!(heard[1].literals, ["foo"]);
                assert!(heard[1].arguments.is_empty());
                assert_eq!(heard[2].literals, ["foo"]);
                assert_eq!(
                    heard[2].argument("value"),
                    Some(&mod_api::CommandArgumentValue::Integer(7))
                );
                assert_eq!(heard[3].literals, ["choice"]);
                assert!(heard[3].argument("a").is_none());
                assert_eq!(
                    heard[3].argument("b"),
                    Some(&mod_api::CommandArgumentValue::String("7".into()))
                );
                assert_eq!(heard[4].literals, ["choice", "bar"]);
                assert_eq!(
                    heard[4].argument("a"),
                    Some(&mod_api::CommandArgumentValue::Integer(7))
                );
                assert_eq!(heard[5].literals, ["choice"]);
                assert_eq!(
                    heard[5].argument("b"),
                    Some(&mod_api::CommandArgumentValue::String("text".into()))
                );
            });
        });
    }

    #[test]
    fn native_executable_permissions_do_not_follow_public_descendants() {
        for optional in [false, true] {
            for reverse in [false, true] {
                let admin = if optional {
                    "foo [value:integer]"
                } else {
                    "foo"
                };
                let mut commands = vec![
                    mod_api::Command::op(admin, 4),
                    mod_api::Command::new("foo bar"),
                ];
                if reverse {
                    commands.reverse();
                }
                let (tree, _, problems) = tree(&commands);
                assert!(problems.is_empty(), "{problems:?}");
                let mut dispatcher = Dispatcher::new();
                register_dispatcher(
                    &mut dispatcher,
                    &tree,
                    "native_mod_command",
                    &HashMap::new(),
                );
                for permission in 0..=4 {
                    let registries = horizon_client::command::Registries {
                        permission,
                        ..Default::default()
                    };
                    for line in ["foo", "foo bar", "foo 7"] {
                        let allowed =
                            line == "foo bar" || permission == 4 && (line == "foo" || optional);
                        assert_eq!(
                            dispatcher.parse(line, &registries).executable().is_ok(),
                            allowed,
                            "{commands:?}, level {permission}, {line}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn native_public_prefixes_do_not_open_admin_children() {
        for reverse in [false, true] {
            let mut commands = vec![
                mod_api::Command::new("foo"),
                mod_api::Command::op("foo bar", 4),
            ];
            if reverse {
                commands.reverse();
            }
            let (tree, _, problems) = tree(&commands);
            assert!(problems.is_empty(), "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            for permission in 0..=4 {
                let registries = horizon_client::command::Registries {
                    permission,
                    ..Default::default()
                };
                assert!(dispatcher.parse("foo", &registries).executable().is_ok());
                assert_eq!(
                    dispatcher
                        .parse("foo bar", &registries)
                        .executable()
                        .is_ok(),
                    permission == 4
                );
            }
        }
    }

    #[test]
    fn native_shared_executable_prefix_takes_its_lowest_usage_level() {
        for reverse in [false, true] {
            let mut commands = vec![
                mod_api::Command::op("foo [value:integer]", 4),
                mod_api::Command::op("foo", 2),
            ];
            if reverse {
                commands.reverse();
            }
            let (tree, _, problems) = tree(&commands);
            assert!(problems.is_empty(), "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            for permission in 0..=4 {
                let registries = horizon_client::command::Registries {
                    permission,
                    ..Default::default()
                };
                assert_eq!(
                    dispatcher.parse("foo", &registries).executable().is_ok(),
                    permission >= 2
                );
                assert_eq!(
                    dispatcher.parse("foo 7", &registries).executable().is_ok(),
                    permission == 4
                );
            }
        }
    }

    #[test]
    fn native_rejected_usage_cannot_open_an_executable_prefix() {
        let (tree, _, problems) = tree(&[
            mod_api::Command::op("foo <value:integer>", 4),
            mod_api::Command::new("foo [value:word]"),
        ]);
        assert_eq!(problems.len(), 1);
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        for permission in 0..=4 {
            let registries = horizon_client::command::Registries {
                permission,
                ..Default::default()
            };
            assert!(dispatcher.parse("foo", &registries).executable().is_err());
            assert!(
                dispatcher
                    .parse("foo hello", &registries)
                    .executable()
                    .is_err()
            );
            assert_eq!(
                dispatcher.parse("foo 7", &registries).executable().is_ok(),
                permission == 4
            );
        }
    }

    #[test]
    fn a_command_takes_its_lowest_level() {
        let (t, levels, _) = tree(&[
            mod_api::Command::new("warp [name:word]"),
            mod_api::Command::op("warp set <name:word>", 2),
        ]);
        assert_eq!(levels["warp"], 0);
        assert!(t["children"]["warp"].get("permissions").is_none());
        let set = &t["children"]["warp"]["children"]["set"];
        assert_eq!(set["permissions"]["permission"]["level"], "gamemasters");
    }

    #[test]
    fn bad_usages_and_vanilla_names_are_left_out() {
        let (t, levels, problems) = tree(&[
            mod_api::Command::new("give <x:integer>"),
            mod_api::Command::new("a [x:integer] <y:integer>"),
            mod_api::Command::new("b <x:text> <y:word>"),
            mod_api::Command::new("c <x:colour>"),
            mod_api::Command::new("<x:integer>"),
            mod_api::Command::new("Bad"),
            mod_api::Command::new("d <x:integer>"),
            mod_api::Command::new("d <x:word>"),
        ]);
        assert_eq!(problems.len(), 7, "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(
            t["children"]["d"]["children"]["x"]["parser"],
            "brigadier:integer"
        );
        let (none, levels, _) = tree(&[]);
        assert!(none.is_null() && levels.is_empty());
    }
}

#[cfg(test)]
pub(super) mod fake_action_tests {
    use super::super::Game;
    use super::{Acting, ActionOutcome, ActionRefused, FakeEffect, PlayerAction};
    use horizon_client::items::ItemStack;
    use mod_api::{BlockHit, Hand, PlayerRef};

    /// A server with a real guest (uuid 1) and a fake (uuid 2) standing at (0.5, 64, 0.5) on
    /// a stone floor, holding `item`, in game mode `mode`: the fake's handle and both conns.
    pub(in crate::game) fn fake(
        game: &mut Game<'_>,
        item: Option<&str>,
        mode: i32,
    ) -> (PlayerRef, u32, u32) {
        super::super::bossbars::audience_tests::add_real_and_fake(game);
        game.options.dedicated = Some(Vec::new());
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in -3..=3 {
            for z in -3..=3 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        let level = game.level(game.dim.id()).unwrap();
        let player = game.fake_player(level, 2, "fake").unwrap();
        let host = game.lan.as_mut().unwrap();
        let conn = |uuid| {
            host.hub
                .guests
                .iter()
                .find(|g| g.uuid == uuid)
                .unwrap()
                .conn
        };
        let (fake, real) = (conn(2), conn(1));
        host.hub.guests.update_all(|g| {
            g.position = [0.5, 64.0, 0.5];
            g.game_mode = mode;
            g.abilities.update_for_mode(mode);
            g.inventory[g.selected] = item.map(|i| ItemStack::new(i, 1));
        });
        (player, fake, real)
    }

    fn place(game: &mut Game<'_>, pos: [i32; 3], state: &str) {
        // `name[key=value,...]`, as a command writes a state.
        let (name, props) = state.split_once('[').unwrap_or((state, "]"));
        let properties = props
            .trim_end_matches(']')
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let state = horizon_world::chunk::BlockState {
            name: name.into(),
            properties,
        };
        let id = game.pack.assets.registry.resolve(&state).unwrap();
        game.world.set(pos[0], pos[1], pos[2], id).unwrap();
    }

    /// A click on the top of the block at `pos`.
    fn on_top(pos: [i32; 3]) -> PlayerAction {
        let [x, y, z] = pos.map(f64::from);
        PlayerAction::UseOn {
            hand: Hand::Main,
            pos,
            hit: BlockHit::new([x + 0.5, y + 1.0, z + 0.5], [0, 1, 0], false),
        }
    }

    use super::super::{
        boats::Boat, minecarts::CART_KEY_Y, minecarts::CartKind, minecarts::Minecart,
        respawn_anchor::RespawnConfig, stations::Kind, stations::Station,
    };

    fn held(game: &Game<'_>, uuid: u128) -> Option<ItemStack> {
        let host = game.lan.as_ref().unwrap();
        let g = host.hub.guests.iter().find(|g| g.uuid == uuid).unwrap();
        g.inventory[g.selected].clone()
    }

    /// A wandering trader at `at`, entity id `id`, and its handle.
    fn trader(game: &mut Game<'_>, id: u32, at: [f64; 3]) -> mod_api::EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:wandering_trader")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(horizon_client::mobs::Mob::new(
            id,
            kind,
            horizon_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }

    /// A fake player's use on a trader frees it again: the session the use
    /// opened closes through the merchant's own close path, so the trader is
    /// not left trading and the next use opens it as the first did.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_trader_a_fake_player_used_trades_with_the_next_one() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:stick"), 0);
            let id = 6000;
            let target = trader(game, id, [2.5, 64.0, 0.5]);
            let interact = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 0.5, 0.0],
            };
            assert_eq!(
                game.player_perform_action(player, interact),
                Ok(ActionOutcome::Consumed)
            );
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            // With no offers the menu never opens, and this test would say nothing.
            let offers = mob.extra.trade.as_ref().map_or(0, |t| t.offers.len());
            assert!(offers > 0, "a wandering trader has offers");
            assert!(!mob.extra.flag2, "the trader is not left trading");
            assert!(game.station.is_none(), "the session is not left open");
            assert_eq!(
                game.player_perform_action(player, interact),
                Ok(ActionOutcome::Consumed),
                "the next use opens it again",
            );
        });
    }

    /// A chest minecart, a hopper minecart and a chest boat: the items stay
    /// on the vehicle, and the menu's stand-in is out of the level, where a
    /// transfer would edit it instead of the vehicle.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_vehicles_chest_a_fake_player_used_keeps_its_items_and_no_stand_in() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, fake_conn, _) = fake(game, None, 0);
            let at = horizon_core::geometry::Vec3::new(2.5, 64.0, 0.5);
            let diamonds = Some(ItemStack::new("minecraft:diamond", 5));
            let mut vehicles = Vec::new();
            for (id, kind) in [(6100, CartKind::Chest), (6101, CartKind::Hopper)] {
                let mut cart = Minecart::new(id, kind, at);
                cart.items[3] = diamonds.clone();
                vehicles.push((id, false));
                game.carts.push(cart);
            }
            let mut boat = Boat::new(6102, "oak".into(), at, 0.0).with_chest();
            boat.items[3] = diamonds.clone();
            // ChestBoat.interact opens the chest for a sneaking player.
            vehicles.push((6102, true));
            game.boats.push(boat);

            for (id, sneaking) in vehicles {
                let target = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
                let interact = PlayerAction::InteractOn {
                    hand: Hand::Main,
                    target,
                    location: [0.0, 0.5, 0.0],
                };
                // Twice: the second use is the next one finding it as it was.
                for _ in 0..2 {
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(fake_conn, |guest| guest.sneaking = sneaking)
                        .unwrap();
                    assert_eq!(
                        game.player_perform_action(player, interact),
                        Ok(ActionOutcome::Consumed)
                    );
                    let key = (i32::MAX - 15, CART_KEY_Y, id as i32);
                    assert!(
                        game.world.block_entities.get(&key).is_none(),
                        "vehicle {id} left a stand-in",
                    );
                    assert!(game.container.is_none());
                    let items = game
                        .carts
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| &c.items)
                        .or_else(|| game.boats.iter().find(|b| b.id == id).map(|b| &b.items))
                        .unwrap();
                    assert_eq!(items[3], diamonds, "vehicle {id} lost its items");
                }
            }
        });
    }

    /// A real guest keeps its vehicle menu over a cart a fake player then
    /// uses: the refused open closes only a stand-in nobody is looking at,
    /// and the staging does not reset what the viewer's menu is editing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_viewer_keeps_the_vehicle_menu_a_fake_player_used() {
        use super::super::minecarts::{CART_KEY_Y, CartKind, Minecart};
        use horizon_client::containers::BlockEntity;
        use horizon_client::lan::menu::Slots;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, fake_conn, real_conn) = fake(game, None, 0);
            let at = horizon_core::geometry::Vec3::new(2.5, 64.0, 0.5);
            let mut cart = Minecart::new(6110, CartKind::Chest, at);
            cart.items[3] = Some(ItemStack::new("minecraft:diamond", 5));
            game.carts.push(cart);
            let gi = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == real_conn)
                .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(real_conn, |guest| guest.position = [0.5, 64.0, 0.5])
                .unwrap();
            assert_eq!(
                game.guest_interact(real_conn, gi, 6110, Hand::Main, [2.5, 64.5, 0.5]),
                Ok(true),
                "the guest's use opens the vehicle's menu",
            );
            let key = (i32::MAX - 15, CART_KEY_Y, 6110);
            let viewer = |game: &Game<'_>| {
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(real_conn)
                    .unwrap()
                    .menu
                    .as_ref()
                    .is_some_and(|m| m.slots == Slots::Block(key))
            };
            assert!(viewer(game), "the guest's use opened the vehicle's menu");
            assert!(game.world.block_entities.contains_key(&key));
            // The viewer's live edit: its menu's clicks write through to the
            // stand-in (`source.write` after every click).
            let emerald = Some(ItemStack::new("minecraft:emerald", 2));
            if let Some(horizon_client::containers::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&key)
            {
                items[4] = emerald.clone();
            }
            let target = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 6110);
            assert_eq!(
                game.player_perform_action(
                    player,
                    PlayerAction::InteractOn {
                        hand: Hand::Main,
                        target,
                        location: [0.0, 0.5, 0.0],
                    },
                ),
                Ok(ActionOutcome::Consumed)
            );
            assert!(viewer(game), "the fake's use kept the viewer's menu");
            assert!(
                game.world.block_entities.contains_key(&key),
                "the fake's use kept the stand-in the menu backs",
            );
            let BlockEntity::Chest(items) = &game.world.block_entities[&key] else {
                unreachable!()
            };
            assert_eq!(items[3], Some(ItemStack::new("minecraft:diamond", 5)));
            assert_eq!(items[4], emerald, "the staging kept the viewer's edit");
            game.guest_close_all(real_conn);
            assert!(!game.world.block_entities.contains_key(&key));
            let items = &game.carts.iter().find(|c| c.id == 6110).unwrap().items;
            assert_eq!(
                items[3],
                Some(ItemStack::new("minecraft:diamond", 5)),
                "the viewer's close put the items back",
            );
            assert_eq!(items[4], emerald, "the viewer's edit reached the vehicle");
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(fake_conn)
                    .unwrap()
                    .menu
                    .is_none()
            );
        });
    }

    /// A fake player's click on a shulker box or an ender chest is refused
    /// at the open: no open count climbs, no stand-in chest enters the level
    /// and no lid opens.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_shulker_box_and_an_ender_chest_a_fake_player_used_are_left_shut() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let shulker = [1, 64, 0];
            place(game, shulker, "minecraft:shulker_box");
            let count =
                |game: &Game<'_>| game.be_anims.shulkers.get(&shulker).map(|s| s.open_count);
            assert_eq!(
                game.player_perform_action(player, on_top(shulker)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert!(
                count(game).is_none_or(|c| c == 0),
                "the use is refused at the open, nothing opened"
            );
            assert!(game.container.is_none());
            assert_eq!(
                game.player_perform_action(player, on_top(shulker)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu)),
                "the next use is refused the same way",
            );
            assert!(
                count(game).is_none_or(|c| c == 0),
                "the count doesn't climb"
            );

            let diamonds = Some(ItemStack::new("minecraft:diamond", 5));
            game.ender_items[0] = diamonds.clone();
            let ender = [2, 64, 0];
            place(game, ender, "minecraft:ender_chest");
            let key = (ender[0], ender[1], ender[2]);
            // A raw block state has no stand-in. Establish the closed state
            // whose preservation this refused action is meant to check.
            game.open_ender_chest(key);
            game.close_ender_chest();
            game.container = None;
            assert_eq!(
                game.player_perform_action(player, on_top(ender)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert!(game.ender_open.is_none());
            match game.world.block_entities.get(&key) {
                Some(horizon_client::chunk::block_entity::BlockEntity::Chest(items)) => assert!(
                    items.iter().all(Option::is_none),
                    "the player's items stayed"
                ),
                other => panic!("the ender chest lost its block entity: {other:?}"),
            }
            assert_eq!(
                game.ender_items[0], diamonds,
                "the server's own ender items changed"
            );
            assert!(!game.lid_openers.contains_key(&key), "its lid stayed open");
        });
    }

    /// Each field holds a value the use can't have written, so a swap that
    /// forgets one fails whether the use changed it or not.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_servers_own_player_state_comes_back_after_a_fake_use() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let dim = game.dim;
            let config = RespawnConfig {
                dim,
                pos: [1, 2, 3],
                yaw: 0.0,
                pitch: 0.0,
                forced: false,
            };
            let spawn = horizon_core::geometry::Vec3::new(1.0, 2.0, 3.0);
            game.station = Some(Station::new_kind(Kind::Anvil, [4, 5, 6]));
            game.craft_table = Some([7, 8, 9]);
            game.experience.level = 7;
            game.respawn_config = Some(config);
            game.spawn = Some(spawn);
            // An ender inventory left open goes back while the fake player
            // acts, and its stand-in returns afterwards, as a guest's turn does.
            let stand_in = (12, 64, 8);
            game.open_ender_chest(stand_in);
            game.container = None;

            place(game, [1, 64, 0], "minecraft:crafting_table");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert_eq!(
                game.station.as_ref().map(|s| (s.kind, s.pos)),
                Some((Kind::Anvil, [4, 5, 6])),
            );
            assert_eq!(game.craft_table, Some([7, 8, 9]));
            assert_eq!(game.experience.level, 7);
            assert_eq!(game.respawn_config, Some(config));
            assert_eq!(game.spawn, Some(spawn));
            assert_eq!(game.ender_open, Some(stand_in));
            assert!(game.world.block_entities.get(&stand_in).is_some());

            let setup = PlayerActionSetup::new(
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:fishing_rod",
                    1,
                ))),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                0.0,
                0.0,
                false,
            );
            let rod = player
                .perform_action_scoped(game, PlayerAction::UseItem { hand: Hand::Main }, setup)
                .unwrap();
            assert_eq!(rod.outcome, ActionOutcome::Consumed, "the rod casts");
            assert!(game.hook.is_none(), "no hook is left for nobody to reel in");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fake_uses_items_through_the_players_own_path() {
        use super::super::native_entity_refs::tests::with_game;
        // A log on stone: placed, one used up.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Consumed)
            );
            let level = game.level(game.dim.id()).unwrap();
            let log = game.level_block_property(level, [1, 64, 0], "axis");
            assert_eq!(log.as_deref(), Some("y"), "placed on a top face, standing");
            assert_eq!(held(game, 2), None);
        });
        // A bucket clicked on a block passes, as vanilla's does (BucketItem has no useOn), and
        // empties when used, by its ray from the eyes (looking straight down here).
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:water_bucket"), 0);
            game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                g.pitch = 90.0;
            });
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Passed)
            );
            let water = PlayerAction::UseItem { hand: Hand::Main };
            assert_eq!(
                game.player_perform_action(player, water),
                Ok(ActionOutcome::Consumed)
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [0, 64, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:water")
            );
            assert_eq!(
                held(game, 2).map(|s| s.item.to_string()).as_deref(),
                Some("minecraft:bucket")
            );
        });
        // A pearl thrown into the air: one used up.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:ender_pearl"), 0);
            let pearl = PlayerAction::UseItem { hand: Hand::Main };
            assert_eq!(
                game.player_perform_action(player, pearl),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(held(game, 2), None);
        });
    }

    /// A zombie standing at `at`, entity id `id`, and its handle.
    fn zombie(game: &mut Game<'_>, id: u32, at: [f64; 3]) -> mod_api::EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(horizon_client::mobs::Mob::new(
            id,
            kind,
            horizon_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }

    fn health(game: &Game<'_>, id: u32) -> f32 {
        game.mobs.iter().find(|m| m.id == id).unwrap().health
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn scoped_fake_use_returns_items_and_restores_persistent_state() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.yaw = 31.0;
                    guest.pitch = 12.0;
                    guest.selected = 3;
                    guest.inventory[3] = Some(ItemStack::new("minecraft:emerald", 4));
                    guest.offhand = Some(ItemStack::new("minecraft:apple", 2));
                    guest.sneaking = true;
                    guest.previous_game_mode = Some(3);
                    guest.motion.as_mut().unwrap().shift_key_down = true;
                })
                .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&conn)
                .unwrap()
                .cooldowns
                .add("minecraft:bucket".into(), 100);
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                5000,
                kind,
                horizon_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                0.0,
            ));
            let target = game.entity(5000).unwrap();
            let setup = PlayerActionSetup::new(
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:bucket",
                    2,
                ))),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                170.0,
                -15.0,
                false,
            );
            let action = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 0.5, 0.0],
            };
            let result = player.perform_action_scoped(game, action, setup).unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held,
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:bucket",
                    1
                )))
            );
            assert_eq!(
                result.other,
                vec![crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:milk_bucket",
                    1
                ))]
            );
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.position, [0.5, 64.0, 0.5]);
            assert_eq!(
                (guest.yaw, guest.pitch, guest.selected, guest.game_mode),
                (31.0, 12.0, 3, 1)
            );
            assert_eq!(guest.previous_game_mode, Some(3));
            assert_eq!(guest.offhand, Some(ItemStack::new("minecraft:apple", 2)));
            assert_eq!(
                guest.inventory[3],
                Some(ItemStack::new("minecraft:emerald", 4))
            );
            assert!(guest.sneaking && guest.motion.as_ref().unwrap().shift_key_down);
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(game.fake_player(level, 2, "fake"), Some(player));
            // The original cooldown is still in force, not the temporary action's timer.
            assert!(
                game.lan.as_ref().unwrap().play[&conn]
                    .cooldowns
                    .is_on_cooldown("minecraft:bucket")
            );
            assert_eq!(game.lan.as_ref().unwrap().hub.guests.len(), 2);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn scoped_fake_use_restores_the_persistent_menu_owner() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            let mut menu = Box::new(horizon_client::menu::Menu::new(
                horizon_client::menu::Kind::Inventory,
            ));
            menu.carried = Some(ItemStack::new("minecraft:stick", 3));
            let owner = (&*menu) as *const horizon_client::menu::Menu;
            *game
                .lan
                .as_mut()
                .unwrap()
                .menus
                .entry(conn)
                .or_default()
                .crafting
                .inventory = Some(menu);
            place(game, [1, 64, 0], "minecraft:chest");
            let setup = mod_api::PlayerActionSetup::new(
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:oak_log",
                    1,
                ))),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                0.0,
                90.0,
                false,
            );
            let result = game
                .player_perform_action_scoped(player, on_top([1, 64, 0]), setup)
                .unwrap();
            assert_eq!(
                result.outcome,
                ActionOutcome::EffectBlocked(FakeEffect::Menu)
            );
            let restored = game.lan.as_ref().unwrap().menus[&conn]
                .crafting
                .inventory
                .as_ref()
                .unwrap();
            assert_eq!((&**restored) as *const horizon_client::menu::Menu, owner);
            assert_eq!(restored.carried, Some(ItemStack::new("minecraft:stick", 3)));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn scoped_fake_use_uses_viewpoint_secondary_use_and_restores_on_refusal() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            let setup = |item, sneaking| {
                PlayerActionSetup::new(
                    Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                        item, 1,
                    ))),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    90.0,
                    sneaking,
                )
            };
            // Air use rays from the supplied eyes/look, not the persistent fake's view.
            let mut viewpoint = setup("minecraft:water_bucket", false);
            viewpoint.eye = [2.5, 64.0 + f64::from(1.62_f32), 2.5];
            viewpoint.yaw = 117.0;
            let result = game
                .player_perform_action_scoped(
                    player,
                    PlayerAction::UseItem { hand: Hand::Main },
                    viewpoint,
                )
                .unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held,
                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                    "minecraft:bucket",
                    1
                )))
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [2, 64, 2])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:water")
            );
            place(game, [1, 64, 0], "minecraft:chest");
            let result = game
                .player_perform_action_scoped(
                    player,
                    on_top([1, 64, 0]),
                    setup("minecraft:oak_log", false),
                )
                .unwrap();
            assert_eq!(
                result.outcome,
                ActionOutcome::EffectBlocked(FakeEffect::Menu)
            );
            let result = game
                .player_perform_action_scoped(
                    player,
                    on_top([1, 64, 0]),
                    setup("minecraft:oak_log", true),
                )
                .unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held, None,
                "survival placement consumes the temporary stack"
            );
            assert_eq!(
                game.level_block(level, [1, 65, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:oak_log")
            );
            assert_eq!(
                game.player_perform_action_scoped(
                    player,
                    on_top([40, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::OutOfReach)
            );
            let mut invalid = setup("minecraft:oak_log", false);
            invalid.eye[0] = f64::NAN;
            assert_eq!(
                game.player_perform_action_scoped(player, on_top([1, 63, 0]), invalid),
                Err(ActionRefused::InvalidSetup)
            );
            let real = game.player(1).unwrap();
            assert_eq!(
                game.player_perform_action_scoped(
                    real,
                    on_top([1, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::NotFake)
            );
            let acting = Acting::start(&game.fake_action);
            assert_eq!(
                game.player_perform_action_scoped(
                    player,
                    on_top([1, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::Busy)
            );
            drop(acting);
            assert_eq!(held(game, 2), Some(ItemStack::new("minecraft:diamond", 1)));
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.game_mode, 1);
            assert_eq!(guest.position, [0.5, 64.0, 0.5]);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fake_entity_interaction_checks_targets_and_does_not_use_items_in_air() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:ender_pearl"), 0);
            let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let far = zombie(game, 5001, [20.0, 64.0, 0.5]);
            let interact = |target| PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0; 3],
            };
            assert_eq!(
                game.player_perform_action(player, interact(near)),
                Ok(ActionOutcome::Passed)
            );
            assert_eq!(
                held(game, 2),
                Some(ItemStack::new("minecraft:ender_pearl", 1))
            );
            assert!(game.creatures.projectiles.is_empty());
            assert_eq!(
                game.player_perform_action(player, interact(far)),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(
                game.player_perform_action(player, interact(player.entity())),
                Err(ActionRefused::InvalidTarget)
            );
            let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), near.id());
            assert_eq!(
                game.player_perform_action(player, interact(stale)),
                Err(ActionRefused::InvalidTarget)
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn scoped_fake_use_hooks_see_the_setup_and_fail_is_not_consumed() {
        use mod_api::{Flow, Hooks, HostSide, PlayerActionSetup, RightClickBlockEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        #[derive(Default)]
        struct Deny(Arc<AtomicBool>);
        #[mod_api::horizon_mod]
        impl Hooks for Deny {
            fn right_click_block(&self, host: HostSide, event: &mut RightClickBlockEvent) -> Flow {
                let HostSide::Server(host) = host else {
                    panic!("server action");
                };
                assert_eq!(event.click.player, 2);
                let entity = host.entity(event.click.player_entity_id as u32).unwrap();
                assert_eq!(entity.rotation(host), Some([123.0, 17.0]));
                assert_eq!(
                    entity.eye_position(host),
                    Some([0.5, 64.0 + f64::from(1.62_f32), 0.5])
                );
                assert_eq!(entity.is_shift_key_down(host), Some(true));
                self.0.store(true, Ordering::Relaxed);
                *event.cancellation_result = mod_api::InteractionResult::Fail;
                Flow::Cancel
            }
        }
        let heard = Arc::new(AtomicBool::new(false));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "deny",
                Box::new(Deny(heard.clone())),
            )]
            .into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let (player, ..) = fake(game, Some("minecraft:diamond"), 0);
                    game.use_failed = true;
                    let result = player
                        .perform_action_scoped(
                            game,
                            on_top([1, 63, 0]),
                            PlayerActionSetup::new(
                                Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                                    "minecraft:oak_log",
                                    1,
                                ))),
                                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                                123.0,
                                17.0,
                                true,
                            ),
                        )
                        .unwrap();
                    assert!(
                        heard.load(Ordering::Relaxed),
                        "normal Host-bearing hook ran"
                    );
                    assert_eq!(result.outcome, ActionOutcome::Passed);
                    assert_eq!(
                        result.held,
                        Some(crate::game::api_values::test_snapshot(&ItemStack::new(
                            "minecraft:oak_log",
                            1
                        )))
                    );
                    assert!(game.use_failed, "outer per-action flag restored");
                    let level = game.level(game.dim.id()).unwrap();
                    assert_eq!(
                        game.level_block(level, [1, 64, 0])
                            .and_then(mod_api::__private::block_key_id),
                        Some("minecraft:air")
                    );
                    assert_eq!(held(game, 2), Some(ItemStack::new("minecraft:diamond", 1)));
                });
            },
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mob_target_setters_refuse_persistent_and_acting_fakes() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            let entity = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let mob = game.mob(entity).unwrap();
            let target = game.living(player.entity()).unwrap();
            assert!(!mob.set_target(game, Some(target)));
            assert_eq!(mob.target(game), None);
            assert_eq!(
                game.as_guest(conn, |g| mob.set_target(g, Some(target))),
                Some(false)
            );
            assert_eq!(mob.target(game), None);
            assert!(!game.lan.as_ref().unwrap().mob_players.contains_key(&5000));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fake_attacks_through_the_players_own_path() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let (player, fake_conn, _) = fake(game, None, 0);
            game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                g.yaw = -90.0;
            });
            // A fist does 1 (the attack damage attribute's base), scaled by the attack strength
            // of a player that hasn't waited (`getAttackStrengthScale(0.5)`: 0.5 of a 5-tick
            // delay, 0.1, so 0.2 + 0.1² × 0.8 = 0.208), then by a zombie's 2 armour
            // (`CombatRules.getDamageAfterAbsorb`: 1 - (2 - 0.208 / 2) / 25).
            let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let full = health(game, 5000);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: near }),
                Ok(ActionOutcome::Consumed)
            );
            let dealt = 0.208f32 * (1.0 - (2.0 - 0.208 / 2.0) / 25.0);
            assert!((full - health(game, 5000) - dealt).abs() < 1e-4);
            // No mob takes the fake as the player to answer.
            let bound = game.lan.as_ref().unwrap().mob_players.get(&5000).copied();
            assert_ne!(bound, Some(fake_conn));
            assert_eq!(bound, None);
            // Ten blocks off is past the attack range; the fake itself isn't a target, nor is
            // an id no entity has.
            let far = zombie(game, 5001, [10.5, 64.0, 0.5]);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: far }),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(health(game, 5001), full);
            assert_eq!(
                game.player_perform_action(
                    player,
                    PlayerAction::Attack {
                        target: player.entity()
                    }
                ),
                Err(ActionRefused::InvalidTarget)
            );
            let nobody = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 6000);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: nobody }),
                Err(ActionRefused::InvalidTarget)
            );
        });
    }

    /// A listener on `attack_entity` records the target's health where the hook
    /// runs; with `cancel` set it vetoes the blow.
    struct AttackWatcher {
        seen: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AttackWatcher {
        fn attack_entity(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::AttackEntityEvent<'_>,
        ) -> mod_api::Flow {
            let health = u32::try_from(event.entity)
                .ok()
                .and_then(|id| host.entity(id))
                .and_then(|entity| host.living(entity))
                .and_then(|living| living.health(host))
                .expect("the watcher's target is a live zombie");
            self.seen.lock().unwrap().push(health);
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }

    /// `attack_entity` runs at `Player.attack`'s start, before the blow and the
    /// swing's attack-strength reset (vanilla's `getAttackStrengthScale(0.5)`
    /// is read there too, and `onAttack` resets the ticker after it). The hook
    /// sees the target untouched, the blow uses the charge that was there when
    /// the hook ran, and a veto leaves both the target and the charge alone.
    /// The charge itself is read through the damage it deals: the read a mod
    /// gets in the hook is `PlayerRef::attack_strength_scale` (part of the same
    /// issue, landed separately).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn attack_entity_runs_before_the_blow_resets_the_attack_charge() {
        use super::super::native_entity_refs::tests::with_game;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        let seen = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "attack-watcher",
            Box::new(AttackWatcher {
                seen: seen.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            // Half a fist's cooldown (delay 5 ticks): the charge is (2 + 0.5) / 5 = 0.5.
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                // A placed fake participates in actions, not online-player tracking.
                // The packet attack gate must still reach the hook and damage below.
                let host = game.lan.as_ref().unwrap();
                let guest = host.hub.guest(fake_conn).unwrap();
                assert!(guest.is_fake());
                assert!(guest.placed);
                assert_eq!(guest.dimension, host.hub.config.dimension);
                assert!(!guest.participates_online());
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
                let full = health(game, 5000);
                game.as_guest(fake_conn, |g| g.attack_ticks = 2).unwrap();
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Consumed)
                );
                assert_eq!(
                    *seen.lock().unwrap(),
                    vec![full],
                    "the hook runs before the blow lands"
                );
                // The hit then uses the charge that was there for the hook: 0.5 scales a fist's
                // 1 to 0.4, and a zombie's 2 armour takes `getDamageAfterAbsorb`'s share.
                let dealt = 0.4f32 * (1.0 - (2.0 - 0.4 / 2.0) / 25.0);
                assert!(
                    (full - health(game, 5000) - dealt).abs() < 1e-4,
                    "the blow uses the pre-reset charge"
                );
                assert_eq!(
                    game.as_guest(fake_conn, |g| g.attack_ticks).unwrap(),
                    0,
                    "the swing resets the charge after the hit"
                );
            });
            // A veto at the hook leaves the target unhurt and the charge where it was: neither
            // the hit nor the reset has run yet.
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5001, [2.0, 64.0, 0.5]);
                let full = health(game, 5001);
                seen.lock().unwrap().clear();
                cancel.store(true, Ordering::Relaxed);
                game.as_guest(fake_conn, |g| g.attack_ticks = 2).unwrap();
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Passed)
                );
                assert_eq!(*seen.lock().unwrap(), vec![full]);
                assert_eq!(health(game, 5001), full, "a veto stops the blow");
                assert_eq!(
                    game.as_guest(fake_conn, |g| g.attack_ticks).unwrap(),
                    2,
                    "and the swing that would reset the charge never starts"
                );
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn attack_strength_rejects_non_finite_partial_without_restricting_finite_ticks() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            game.as_guest(conn, |g| g.attack_ticks = 2).unwrap();
            for partial in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(game.player_attack_strength_scale(player, partial), None);
                game.as_guest(conn, |g| {
                    assert_eq!(g.attack_strength_scale_at(partial), None);
                    assert_eq!(g.player_attack_strength_scale(player, partial), None);
                })
                .unwrap();
            }
            for (partial, expected) in [(-1.0, 0.2), (0.5, 0.5), (2.0, 0.8)] {
                assert_eq!(
                    game.player_attack_strength_scale(player, partial),
                    Some(expected)
                );
                game.as_guest(conn, |g| {
                    assert_eq!(g.attack_strength_scale_at(partial), Some(expected));
                    assert_eq!(
                        g.player_attack_strength_scale(player, partial),
                        Some(expected)
                    );
                })
                .unwrap();
            }
        });
    }

    /// A listener on `attack_entity` reads the attacking player's attack
    /// strength where the hook runs.
    struct ChargeWatcher(std::sync::Arc<std::sync::Mutex<Vec<Option<f32>>>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ChargeWatcher {
        fn attack_entity(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _event: &mut mod_api::AttackEntityEvent<'_>,
        ) -> mod_api::Flow {
            let player = host.players().acting_player().expect("an attacking player");
            let scale = player.attack_strength_scale(host, 0.5);
            self.0.lock().unwrap().push(scale);
            mod_api::Flow::Continue
        }
    }

    /// `PlayerRef::attack_strength_scale` inside `attack_entity` reads the
    /// charge the blow uses: `Player.attack` reads `getAttackStrengthScale`
    /// itself at its start, and the hook runs before that read and before the
    /// swing resets the ticker. After the blow the same read sees 0. A
    /// resting player reads its saved ticker and body too.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn attack_strength_scale_reads_the_charge_at_the_hook_and_not_after_the_swing() {
        use super::super::native_entity_refs::tests::with_game;
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "charge-watcher",
            Box::new(ChargeWatcher(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (player, fake_conn, _) = fake(game, None, 0);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| g.yaw = -90.0);
                let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
                let full = health(game, 5000);
                // A fist's cooldown is 5 ticks: a full ticker reads 1.0, and half of one
                // ((2 + 0.5) / 5) reads 0.5.
                game.as_guest(fake_conn, |g| g.attack_ticks = 5).unwrap();
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(1.0),
                    "a full ticker is a fully charged blow"
                );
                game.as_guest(fake_conn, |g| g.attack_ticks = 2).unwrap();
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(0.5),
                    "a resting fake reads its saved ticker and body"
                );
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::Attack { target: near }),
                    Ok(ActionOutcome::Consumed)
                );
                assert_eq!(*seen.lock().unwrap(), vec![Some(0.5)]);
                // The blow uses the charge the hook read: 0.5 scales a fist's 1 to 0.4, and a
                // zombie's 2 armour takes `getDamageAfterAbsorb`'s share.
                let dealt = 0.4f32 * (1.0 - (2.0 - 0.4 / 2.0) / 25.0);
                assert!((full - health(game, 5000) - dealt).abs() < 1e-4);
                assert_eq!(
                    player.attack_strength_scale(game, 0.5),
                    Some(0.1),
                    "the swing reset the ticker to 0 after the blow: (0 + 0.5) / 5"
                );
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fake_breaks_blocks_at_once_by_the_game_modes_rules() {
        use super::super::native_entity_refs::tests::with_game;
        let level_block = |game: &mut Game<'_>, pos| {
            let level = game.level(game.dim.id()).unwrap();
            game.level_block(level, pos)
        };
        // Survival, bare-handed: stone breaks at once, with no digging time.
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let pos = [1, 63, 0];
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(
                level_block(game, pos).and_then(mod_api::__private::block_key_id),
                Some("minecraft:air")
            );
            // Air breaks nothing; bedrock, which no digging breaks (destroy time -1), stays;
            // ten blocks off is out of reach.
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                Ok(ActionOutcome::Passed)
            );
            place(game, [-1, 63, 0], "minecraft:bedrock");
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [-1, 63, 0] }),
                Ok(ActionOutcome::Passed)
            );
            assert_eq!(
                level_block(game, [-1, 63, 0]).and_then(mod_api::__private::block_key_id),
                Some("minecraft:bedrock")
            );
            place(game, [3, 64, 3], "minecraft:stone");
            game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                g.position = [-6.5, 64.0, -6.5];
            });
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [3, 64, 3] }),
                Err(ActionRefused::OutOfReach)
            );
        });
        // Creative breaks what survival can't dig.
        with_game(|game| {
            let (player, ..) = fake(game, None, 1);
            place(game, [1, 63, 0], "minecraft:bedrock");
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [1, 63, 0] }),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(
                level_block(game, [1, 63, 0]).and_then(mod_api::__private::block_key_id),
                Some("minecraft:air")
            );
        });
        // Adventure mode can't break a block (blockActionRestricted), and a sword in creative
        // breaks nothing (canDestroyBlock).
        for (mode, item) in [(2, None), (1, Some("minecraft:iron_sword"))] {
            with_game(|game| {
                let (player, ..) = fake(game, item, mode);
                let pos = [1, 63, 0];
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                    Ok(ActionOutcome::Passed),
                    "game mode {mode}, holding {item:?}"
                );
                assert_eq!(
                    level_block(game, pos).and_then(mod_api::__private::block_key_id),
                    Some("minecraft:stone")
                );
            });
        }
    }

    /// `as_guest_at` starts a turn only for the guest it names: another `conn` at that place,
    /// a place past the list's end, and a guest already in a turn are refused, not a panic.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_turn_at_a_place_is_refused_for_another_guest() {
        use super::super::lan_actions::TurnRefused;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (_, fake_conn, real_conn) = fake(game, None, 0);
            let gi = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == fake_conn)
                .unwrap();
            assert_eq!(
                game.as_guest_at(gi, real_conn, |_| ()),
                Err(TurnRefused::Gone)
            );
            assert_eq!(
                game.as_guest_at(999, fake_conn, |_| ()),
                Err(TurnRefused::Gone)
            );
            let nested = game.as_guest_at(gi, fake_conn, |g| g.as_guest_at(gi, fake_conn, |_| ()));
            assert_eq!(nested, Ok(Err(TurnRefused::Nested)));
            assert_eq!(game.as_guest_at(gi, fake_conn, |_| 7), Ok(7));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fakes_effects_are_stopped_where_they_start() {
        use super::super::native_entity_refs::tests::with_game;
        // A chest's menu (Game::open_menu).
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            place(game, [1, 64, 0], "minecraft:chest");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
        });
        // A bed (Game::try_sleep).
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            place(
                game,
                [1, 64, 0],
                "minecraft:red_bed[facing=north,part=foot]",
            );
            place(
                game,
                [1, 64, -1],
                "minecraft:red_bed[facing=north,part=head]",
            );
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Sleep))
            );
        });
        // A spectator's chest, the menu that bypasses Game::open_menu (spectator_open_menu).
        with_game(|game| {
            let (player, ..) = fake(game, None, 3);
            place(game, [1, 64, 0], "minecraft:chest");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
        });
        // A sign placed opens no editor (open_sign_editor), but the sign is placed.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_sign"), 0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [1, 64, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:oak_sign")
            );
        });
        // Boarding (native_may_board, every boarding path's gate): refused for the fake only.
        with_game(|game| {
            let (_, fake_conn, real_conn) = fake(game, None, 0);
            assert_eq!(
                game.as_guest(fake_conn, |g| g.native_may_board(99)),
                Some(false)
            );
            assert_eq!(
                game.as_guest(real_conn, |g| g.native_may_board(99)),
                Some(true)
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fakes_action_is_refused_before_anything_runs() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:water_bucket"), 0);
            // Out of reach: nothing used, the bucket still full.
            let far = PlayerAction::UseOn {
                hand: Hand::Main,
                pos: [40, 63, 0],
                hit: BlockHit::new([40.5, 64.0, 0.5], [0, 1, 0], false),
            };
            assert_eq!(
                game.player_perform_action(player, far),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(
                held(game, 2).map(|s| s.item.to_string()).as_deref(),
                Some("minecraft:water_bucket")
            );
            // A real player's handle.
            let real = game.player(1).unwrap();
            assert_eq!(
                game.player_perform_action(real, on_top([1, 63, 0])),
                Err(ActionRefused::NotFake)
            );
            // Inside another action, as a hook running during it would ask.
            let acting = Acting::start(&game.fake_action);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Err(ActionRefused::Busy)
            );
            drop(acting);
            assert!(!game.fake_action.acting());
        });
    }

    /// 1, 64 and 256 fakes each clicking the stone floor with an empty hand every tick, the full
    /// admission and use path (vanilla passes). Run with `-- --ignored --nocapture`, in release.
    #[test]
    #[ignore = "timing"]
    fn fakes_acting_each_tick_cost() {
        #[derive(Clone, Copy, Debug)]
        enum Kind {
            UseOn,
            Attack,
            BreakBlock,
        }
        for kind in [Kind::UseOn, Kind::Attack, Kind::BreakBlock] {
            for count in [1u128, 64, 256] {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let (first, ..) = fake(game, None, 0);
                    let level = game.level(game.dim.id()).unwrap();
                    let mut players = vec![first];
                    for i in 1..count {
                        players.push(
                            game.fake_player(level, 100 + i, &format!("fake{i}"))
                                .unwrap(),
                        );
                    }
                    game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
                        g.position = [0.5, 64.0, 0.5];
                        g.yaw = -90.0;
                    });
                    let target = zombie(game, 5000, [2.0, 64.0, 0.5]);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let (action, expected) = match kind {
                        Kind::UseOn => (on_top([1, 63, 0]), ActionOutcome::Passed),
                        Kind::Attack => (PlayerAction::Attack { target }, ActionOutcome::Consumed),
                        Kind::BreakBlock => (
                            PlayerAction::BreakBlock { pos: [1, 63, 0] },
                            ActionOutcome::Consumed,
                        ),
                    };
                    let ticks = 200;
                    let start = std::time::Instant::now();
                    for _ in 0..ticks {
                        // The zombie never dies, and each break has a stone to break.
                        game.mobs[0].health = 20.0;
                        for &player in &players {
                            if let Kind::BreakBlock = kind {
                                game.world.set(1, 63, 0, stone).unwrap();
                            }
                            let outcome = game.player_perform_action(player, action);
                            assert_eq!(outcome, Ok(expected));
                        }
                    }
                    let elapsed = start.elapsed();
                    let per_tick = elapsed / ticks;
                    let per_action = elapsed / (ticks * players.len() as u32);
                    eprintln!(
                        "fake_action kind={kind:?} players={count} ns_per_action={}",
                        per_action.as_nanos()
                    );
                    println!(
                        "{kind:?}, {count} fakes: {per_tick:?} a tick, {per_action:?} an action"
                    );
                });
            }
        }
    }

    #[test]
    fn the_acting_mark_clears_on_a_panic_and_each_action_starts_unblocked() {
        let action = std::sync::Arc::new(super::FakeAction::default());
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _acting = Acting::start(&action);
            panic!("a mod's hook panics");
        }));
        assert!(unwound.is_err());
        assert!(!action.acting());
        {
            let acting = Acting::start(&action);
            action.block(FakeEffect::Sleep);
            action.block(FakeEffect::Menu);
            // The first effect stopped is the answer.
            assert_eq!(acting.blocked(), Some(FakeEffect::Sleep));
        }
        assert_eq!(Acting::start(&action).blocked(), None);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fakes_action_leaves_the_per_action_state_as_it_was() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            let before = super::super::Hit {
                pos: [7, 7, 7],
                face: [1, 0, 0],
                point: [7.0, 7.5, 7.5],
                inside: true,
            };
            game.forced_hit = Some(before);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Consumed)
            );
            let after = game.forced_hit.expect("the forced hit is put back");
            assert_eq!(
                (after.pos, after.face, after.point, after.inside),
                (before.pos, before.face, before.point, before.inside)
            );
            assert!(game.guest_use.is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_real_player_is_unaffected_at_every_effect_site() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (_, _, real_conn) = fake(game, None, 0);
            for effect in [
                FakeEffect::Menu,
                FakeEffect::Sleep,
                FakeEffect::Ride,
                FakeEffect::ChangeDimension,
            ] {
                assert_eq!(
                    game.as_guest(real_conn, |g| g.fake_blocks(effect)),
                    Some(false)
                );
            }
            // A real guest's click on a chest is the vanilla click, nothing stopped.
            place(game, [1, 64, 0], "minecraft:chest");
            let gi = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == real_conn)
                .unwrap();
            let hit = super::super::Hit {
                pos: [1, 64, 0],
                face: [0, 1, 0],
                point: [1.5, 65.0, 0.5],
                inside: false,
            };
            assert_eq!(
                game.guest_use_item_on(real_conn, gi, hit, Hand::Main),
                Ok(true)
            );
            assert_eq!(
                game.as_guest(real_conn, |g| g.native_may_board(99)),
                Some(true)
            );
        });
    }
}

#[cfg(test)]
mod player_rule_tests {
    use super::super::native_guest_turn_tests::with_guests;
    use super::*;

    /// The guests of `with_guests` are rows 0 and 1; both stand in the level in place.
    fn in_this_level(game: &mut Game<'_>) {
        let level = game.dim.id().to_owned();
        game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
            g.dimension = level.clone();
        });
    }

    fn at(dimension: &str, [x, y, z]: [i32; 3]) -> GlobalPos {
        GlobalPos::new(
            Identifier::parse(dimension).unwrap(),
            BlockPos::new(x, y, z),
        )
    }

    fn respawn(dimension: &str, pos: [i32; 3], yaw: f32, forced: bool) -> PlayerRespawnConfig {
        PlayerRespawnConfig {
            position: at(dimension, pos),
            yaw,
            forced,
        }
    }

    /// `/spawnpoint`'s write and `getRespawnConfig`'s read, on each player's own record.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_respawn_point_is_read_and_written_in_the_players_own_record() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(a.respawn_config(&*game), Some(None), "none until set");

            // The yaw wraps as `RespawnData.of` does: 270 degrees is -90.
            let nether = "minecraft:the_nether";
            assert!(a.set_respawn_config(game, Some(respawn(nether, [1, 2, 3], 270.0, true))));
            assert_eq!(
                a.respawn_config(&*game),
                Some(Some(respawn(nether, [1, 2, 3], -90.0, true)))
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].respawn_elsewhere,
                Some((nether.into(), [1, 2, 3], -90.0, 0.0, true)),
                "the record a respawn reads"
            );
            assert_eq!(
                b.respawn_config(&*game),
                Some(None),
                "the other is untouched"
            );

            // The acting player reads the point swapped into place.
            let seen = game.as_guest(12, |game| a.respawn_config(&*game)).unwrap();
            assert_eq!(seen, Some(Some(respawn(nether, [1, 2, 3], -90.0, true))));

            // Refused writes change nothing: a dimension the game does not have, a yaw that is not a number.
            let nowhere = respawn("test:nowhere", [9, 9, 9], 0.0, false);
            assert!(!a.set_respawn_config(game, Some(nowhere)));
            let not_a_number = respawn(nether, [9, 9, 9], f32::NAN, false);
            assert!(!a.set_respawn_config(game, Some(not_a_number)));
            assert_eq!(
                a.respawn_config(&*game),
                Some(Some(respawn(nether, [1, 2, 3], -90.0, true)))
            );

            // None forgets it.
            assert!(a.set_respawn_config(game, None));
            assert_eq!(a.respawn_config(&*game), Some(None));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].respawn_elsewhere,
                None
            );
        });
    }

    /// The write goes through `spawn_point_change`, so a mod's `player_set_spawn` veto refuses it, as it does `/spawnpoint`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mods_veto_refuses_the_respawn_write() {
        struct Veto;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn player_set_spawn(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mut mod_api::PlayerSetSpawnEvent,
            ) -> mod_api::Flow {
                mod_api::Flow::Cancel
            }
        }
        let mods = vec![mod_api::ModEntry::new("veto", Box::new(Veto))].into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let a = game.player(12).unwrap();
                let overworld = "minecraft:overworld";
                let config = respawn(overworld, [1, 2, 3], 0.0, false);
                assert!(!a.set_respawn_config(game, Some(config)));
                assert_eq!(a.respawn_config(&*game), Some(None));
            });
        });
    }

    /// The client's text-filtering option, as its `ClientInformation` said.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn text_filtering_is_what_the_clients_information_said() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.info.text_filtering = true;
            });
            assert_eq!(a.text_filtering(&*game), Some(true));
            assert_eq!(b.text_filtering(&*game), Some(false));
            // A later `ClientInformation` replaces it.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.info.text_filtering = false;
            });
            assert_eq!(a.text_filtering(&*game), Some(false));
        });
    }

    /// The sleep reads and the counter write, with the night skip's own counts as the oracle.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sleep_eligibility_and_the_timer_follow_the_players_state() {
        with_guests(|game| {
            in_this_level(game);
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let (active, sleeping, deep) = game.sleep_counts();
            assert_eq!(a.sleep_timer(&*game), Some(0));
            assert_eq!(a.counts_for_sleep(&*game), Some(true));
            assert_eq!(a.sleep_ignored(&*game), Some(false));

            // A player in bed at 100 ticks is one of the deep sleepers the night waits for.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.bed = Some([5, 64, 5]);
            });
            assert!(a.set_sleep_timer(game, 100));
            assert_eq!(a.sleep_timer(&*game), Some(100));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].sleep_timer, 100);
            assert_eq!(game.sleep_counts(), (active, sleeping + 1, deep + 1));
            // The wake-up fade's last value is allowed; one past it is not, and changes nothing.
            assert!(a.set_sleep_timer(game, 109));
            assert!(!a.set_sleep_timer(game, 110));
            assert_eq!(a.sleep_timer(&*game), Some(109));

            // Ignoring leaves an out-of-bed player out of the count, and an in-bed one in.
            assert!(b.set_sleep_ignored(game, true));
            assert_eq!(b.sleep_ignored(&*game), Some(true));
            assert_eq!(b.counts_for_sleep(&*game), Some(false));
            assert_eq!(game.sleep_counts().0, active - 1);
            assert!(a.set_sleep_ignored(game, true));
            assert_eq!(a.counts_for_sleep(&*game), Some(true), "in bed");
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.bed = None);
            assert_eq!(a.counts_for_sleep(&*game), Some(false), "out of bed");
            assert!(a.set_sleep_ignored(game, false));
            assert!(b.set_sleep_ignored(game, false));
            assert_eq!(a.counts_for_sleep(&*game), Some(true));

            // A spectator is never counted.
            game.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                g.game_mode = 3;
            });
            assert_eq!(b.counts_for_sleep(&*game), Some(false));

            // The acting player's state is the one in place.
            let seen = game
                .as_guest(12, |game| {
                    (a.sleep_timer(&*game), a.counts_for_sleep(&*game))
                })
                .unwrap();
            assert_eq!(seen, (Some(109), Some(true)));
        });
    }
}

// Dedicated network settings stay engine-private; never expose the forwarding secret.
#[allow(dead_code)]
impl Game<'_> {
    /// Whether the dedicated server runs in online mode (`online-mode` of server.properties):
    /// its players sign in with their accounts. None off a dedicated server and before it listens.
    /// API @API:server_settings@.
    pub(super) fn native_online_mode(&self) -> Option<bool> {
        Some(online_mode_of(&self.dedicated_lan()?.shared.config))
    }
    /// The port the dedicated server listens on: `server-port` of server.properties, or the
    /// `--port` it was started with. None off a dedicated server and before it listens.
    /// API @API:server_settings@.
    pub(super) fn native_server_port(&self) -> Option<u16> {
        Some(self.dedicated_lan()?.port)
    }
    /// How the dedicated server takes its players from a proxy: `"velocity"` when it signs the
    /// forwarded player information with `forwarding.secret` (offline mode behind Velocity),
    /// `"none"` when it doesn't; None off a dedicated server and before it listens. The secret
    /// itself is never returned. API @API:server_settings@.
    pub(super) fn native_proxy_forwarding(&self) -> Option<&'static str> {
        Some(proxy_forwarding_of(&self.dedicated_lan()?.shared.config))
    }
    /// The listening socket of a dedicated server; None on any other side and before it listens.
    fn dedicated_lan(&self) -> Option<&horizon_client::lan::Lan> {
        self.options.dedicated.as_ref()?;
        Some(&self.lan.as_ref()?.hub.lan)
    }
}

/// The online mode a dedicated server's network config carries: it has an
/// `OnlineMode` key (and so signs its logins) only in online mode.
fn online_mode_of(config: &horizon_client::lan::NetConfig) -> bool {
    config.online.is_some()
}

/// The proxy forwarding a dedicated server's network config carries: the secret
/// is set only offline behind a proxy (game/dedicated.rs `forwarding_secret`).
fn proxy_forwarding_of(config: &horizon_client::lan::NetConfig) -> &'static str {
    if config.forwarding_secret.is_some() {
        "velocity"
    } else {
        "none"
    }
}

#[cfg(test)]
mod network_settings_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn server_settings_follow_the_network_config() {
        use horizon_client::lan::{NetConfig, OnlineMode};
        let mut config = NetConfig::loopback(25565, String::new(), 20);
        // online-mode=false, no proxy: no key, no forwarding secret.
        assert!(!online_mode_of(&config));
        assert_eq!(proxy_forwarding_of(&config), "none");
        // online-mode=false behind Velocity: the secret is set.
        config.forwarding_secret = Some(b"secret".to_vec());
        assert!(!online_mode_of(&config));
        assert_eq!(proxy_forwarding_of(&config), "velocity");
        // online-mode=true: the server's key, and forwarding stays off.
        config.forwarding_secret = None;
        config.online = Some(std::sync::Arc::new(OnlineMode::generate(false).unwrap()));
        assert!(online_mode_of(&config));
        assert_eq!(proxy_forwarding_of(&config), "none");
    }
}
