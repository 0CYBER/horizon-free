//! Server commands and players for native mods (docs/native-mods.md,
//! "Server commands and players", API 8): the commands mods add to the
//! server's command tree (`native_register_commands`), a player's command
//! reaching them (`native_perform_command`), and the helpers a server mod
//! answers with: the players online, a line to one of them or to all, the
//! command's feedback and the world's folder; and (API 9) the acting
//! player's plain items: count, take, give. They act on the server's own
//! players (the hub's: a dedicated server's, a LAN host's, single player's
//! server thread's), never on a client's.
use super::{Dim, Game, commands::execute, lan::LanHost};
use horizon_client::{items::ItemStack, lan::data::component_nbt, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::PathBuf};

/// A player on the server, as a server hook sees them.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeServerPlayer {
    pub(crate) uuid: u128,
    pub(crate) name: String,
    /// The operator level, 0 to 4 (`ops.json`; single player's owner with
    /// cheats on is 4).
    pub(crate) permission: u8,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub(crate) dimension: String,
    pub(crate) position: [f64; 3],
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub(crate) game_mode: i32,
    /// Health points, 20 at full; 0 once dead. API 21.
    pub(crate) health: f32,
}

/// A command a native mod adds, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`,
/// `item`, `block_pos`. Usages of one command share their words; the
/// server merges them into one tree.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeCommand {
    pub(crate) usage: String,
    /// The operator level a player needs to see and run it (0: everyone).
    pub(crate) level: u8,
}

#[allow(dead_code)]
impl NativeCommand {
    /// A usage every player may run.
    pub(crate) fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
        }
    }
    /// The same, for operators of `level` and above.
    pub(crate) fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
        }
    }
}

/// The brigadier parser of an argument kind, and its properties.
fn parser(kind: &str) -> Option<(&'static str, Value)> {
    Some(match kind {
        "word" => ("brigadier:string", json!({"type": "word"})),
        "text" => ("brigadier:string", json!({"type": "greedy"})),
        "integer" => ("brigadier:integer", Value::Null),
        "number" => ("brigadier:double", Value::Null),
        "boolean" => ("brigadier:bool", Value::Null),
        "player" => (
            "minecraft:entity",
            json!({"amount": "single", "type": "players"}),
        ),
        "item" => ("minecraft:item_stack", Value::Null),
        "block_pos" => ("minecraft:block_pos", Value::Null),
        _ => return None,
    })
}

/// The report's name of an operator level (`commands.json`'s
/// `permissions`).
const LEVELS: [&str; 5] = ["", "moderators", "gamemasters", "admins", "owners"];

/// One parsed word of a usage: its key in the tree and its node, without
/// children.
fn usage_nodes(usage: &str) -> Result<Vec<(String, Value, bool)>, String> {
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
        let Some((parser, properties)) = parser(kind) else {
            return Err(format!("`{usage}`: unknown kind `{kind}`"));
        };
        optional_seen |= optional;
        greedy_seen |= kind == "text";
        let mut node = json!({"type": "argument", "parser": parser});
        if !properties.is_null() {
            node["properties"] = properties;
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
/// each command's level: the lowest of its usages. A node is runnable where
/// only optional arguments follow it. A command vanilla has (`vanilla`
/// answers) or a usage that doesn't parse is left out, with why.
pub(super) fn command_tree(
    commands: &[NativeCommand],
    vanilla: impl Fn(&str) -> bool,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    let mut root = Map::new();
    let mut levels: HashMap<String, u8> = HashMap::new();
    let mut problems = Vec::new();
    for command in commands {
        let nodes = match usage_nodes(&command.usage) {
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
        let mut children = &mut root;
        for (i, (name, node, _)) in nodes.iter().enumerate() {
            let runnable = nodes.get(i + 1).is_none_or(|n| n.2);
            let entry = children.entry(name.clone()).or_insert_with(|| {
                let mut n = node.clone();
                n["executable"] = false.into();
                n["level"] = 4.into();
                n
            });
            if entry["type"] != node["type"] || entry["parser"] != node["parser"] {
                problems.push(format!(
                    "`{}`: `{name}` is another kind in an earlier usage",
                    command.usage
                ));
                break;
            }
            if runnable {
                entry["executable"] = true.into();
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

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The players on this server, in the order they joined. Empty on a
    /// client and in a world nobody else can join.
    pub(super) fn native_server_players(&self) -> Vec<NativeServerPlayer> {
        let Some(host) = &self.lan else {
            return Vec::new();
        };
        host.hub
            .guests
            .iter()
            .filter(|g| g.placed)
            .map(|g| NativeServerPlayer {
                uuid: g.uuid,
                name: g.name.clone(),
                permission: g.permission,
                dimension: g.dimension.clone(),
                position: g.position,
                game_mode: g.game_mode,
                health: g.health,
            })
            .collect()
    }
    /// A system line in that player's chat; false when they aren't on.
    pub(super) fn native_tell(&self, uuid: u128, line: Text) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.placed && g.uuid == uuid) else {
            return false;
        };
        host.hub.send_to(g.conn, &system_chat(&line));
        true
    }
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player (`native_tell` reaches nobody
    /// there); false when they aren't on. API 21.
    pub(super) fn native_action_bar(&mut self, uuid: u128, line: Text) -> bool {
        let packet = C::SystemChat(play::SystemChat {
            content: component_nbt(&line.component()),
            overlay: true,
        });
        if self.lan_acting_uuid == Some(uuid) {
            self.guest_packets.push(packet);
            return true;
        }
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.placed && g.uuid == uuid) else {
            return false;
        };
        host.hub.send_to(g.conn, &packet);
        true
    }
    /// A system line in every player's chat and the server's log.
    pub(super) fn native_broadcast(&self, line: Text) {
        let plain = line.plain();
        if self.options.dedicated.is_some() {
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
    /// A line of the command's answer, inside `native_perform_command`:
    /// sent to the player whose command it is after the call, as vanilla
    /// commands' feedback is.
    pub(super) fn native_feedback(&mut self, line: Text) {
        match &mut self.chat_capture {
            Some(lines) => lines.push(line),
            None => self.system_message(line),
        }
    }
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (`native_perform_command`, `native_right_click_block`,
    /// `native_use_item_on`, `native_destroy_block`...): None outside such a
    /// hook. API 21.
    pub(super) fn native_acting_player(&self) -> Option<NativeServerPlayer> {
        let p = self.player.as_ref()?;
        Some(NativeServerPlayer {
            uuid: self.lan_acting_uuid?,
            name: self.lan_acting_name.clone()?,
            permission: self.lan_acting_permission?,
            dimension: self.dim.id().to_owned(),
            position: [p.position.x, p.position.y, p.position.z],
            game_mode: self.mode_id(),
            health: p.health.health(),
        })
    }
    /// Runs `line` (no slash) as the server's console does: permission
    /// level 4, from the overworld's spawn, with no output. A teleport to
    /// another level (`execute in minecraft:the_nether run tp Alex 0 70 0`)
    /// happens at the next tick's start, as the console's. Whether it ran
    /// without failing. Only where no player's state is swapped in
    /// (`native_tick`, `native_menu_clicked`): false inside a player's hook,
    /// where the other players are out of reach. API 21.
    pub(super) fn native_run_command(&mut self, line: &str) -> bool {
        if self.lan_acting.is_some() || self.lan.is_none() {
            return false;
        }
        let server = execute::Source {
            dimension: Dim::Overworld,
            ..self.server_source()
        };
        let outer = self.command_source.replace(server);
        let ok = self.as_console(|g| g.run_command(line));
        self.command_source = outer;
        ok
    }
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    pub(super) fn native_world_dir(&self) -> Option<PathBuf> {
        (!self.world_root.as_os_str().is_empty()).then(|| self.world_root.clone())
    }
}

/// Plain stacks: no components beyond the count (no damage, name,
/// enchantment...), the only ones the item helpers count, take or give, so
/// a mod never prices a worn or enchanted item as a new one.
fn plain(stack: &ItemStack) -> bool {
    stack.damage == 0 && stack.extra.is_none()
}

/// The slots the item helpers look at, in `Inventory.add`'s order: the
/// selected slot, the off hand, then the hotbar and the main inventory.
fn order(selected: usize) -> impl Iterator<Item = usize> {
    std::iter::once(selected)
        .chain(std::iter::once(OFFHAND))
        .chain((0..36).filter(move |&i| i != selected))
}
const OFFHAND: usize = 36;

/// The 36 inventory slots and the off hand as one list ([`OFFHAND`] last).
fn slots<'a>(
    inventory: &'a mut [Option<ItemStack>],
    offhand: &'a mut Option<ItemStack>,
) -> Vec<&'a mut Option<ItemStack>> {
    inventory
        .iter_mut()
        .take(36)
        .chain(std::iter::once(offhand))
        .collect()
}

/// How many of plain `item` the slots hold.
fn count_in(slots: &[&mut Option<ItemStack>], item: &str) -> u32 {
    slots
        .iter()
        .filter_map(|s| s.as_ref())
        .filter(|s| &*s.item == item && plain(s))
        .map(|s| s.count)
        .sum()
}

/// Takes up to `count` plain `item`, selected slot first; what it took.
fn take_from(
    mut slots: Vec<&mut Option<ItemStack>>,
    selected: usize,
    item: &str,
    count: u32,
) -> u32 {
    let mut taken = 0;
    for i in order(selected) {
        let slot = &mut *slots[i];
        if let Some(s) = slot
            && &*s.item == item
            && plain(s)
        {
            let n = s.count.min(count - taken);
            s.count -= n;
            taken += n;
            if s.count == 0 {
                *slot = None;
            }
        }
        if taken == count {
            break;
        }
    }
    taken
}

/// How many plain `item` (stacking to `max`) still fit.
fn room_in(slots: &[&mut Option<ItemStack>], item: &str, max: u32) -> u32 {
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| match s.as_ref() {
            // getFreeSlot: never into an empty off hand.
            None if i == OFFHAND => 0,
            None => max,
            Some(s) if &*s.item == item && plain(s) => max.saturating_sub(s.count),
            Some(_) => 0,
        })
        .sum()
}

/// Adds up to `count` plain `item`, onto its stacks first
/// (`getSlotWithRemainingSpace`: the off hand's too), then into empty slots
/// of the inventory (`getFreeSlot`), in [`order`]; what it added. Nothing
/// is dropped.
fn add_to(
    mut slots: Vec<&mut Option<ItemStack>>,
    selected: usize,
    item: &str,
    max: u32,
    count: u32,
) -> u32 {
    let mut added = 0;
    for pass in 0..2 {
        for i in order(selected) {
            let slot = &mut *slots[i];
            let n = match slot {
                Some(s) if pass == 0 && &*s.item == item && plain(s) => {
                    let n = max.saturating_sub(s.count).min(count - added);
                    s.count += n;
                    n
                }
                None if pass == 1 && i != OFFHAND => {
                    let n = max.min(count - added);
                    *slot = Some(ItemStack {
                        item: item.into(),
                        count: n,
                        damage: 0,
                        extra: None,
                    });
                    n
                }
                _ => 0,
            };
            added += n;
            if added == count {
                return added;
            }
        }
    }
    added
}

/// The acting player's items, for server mods (API 9): the player whose
/// action or command a hook hears (`native_perform_command`), as their
/// state is swapped in. Only plain stacks count; the inventory's 36 slots
/// and the off hand, never the armour. Each change reaches the player's
/// screen as any change to their inventory does.
#[allow(dead_code)]
impl Game<'_> {
    /// The main hand's stack: its item, count, and whether it is plain.
    pub(super) fn native_held(&self) -> Option<(String, u32, bool)> {
        let s = self.inventory.get(self.selected)?.as_ref()?;
        Some((s.item.to_string(), s.count, plain(s)))
    }
    /// How many of plain `item` (`minecraft:diamond`) the player carries.
    pub(super) fn native_count_items(&mut self, item: &str) -> u32 {
        count_in(&slots(&mut self.inventory, &mut self.offhand), item)
    }
    /// Every plain item the player carries, with how many, by id.
    pub(super) fn native_carried_items(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = Vec::new();
        let held = self
            .inventory
            .iter()
            .take(36)
            .chain(std::iter::once(&self.offhand));
        for s in held.flatten().filter(|s| plain(s)) {
            match out.iter_mut().find(|(i, _)| *i == *s.item) {
                Some((_, n)) => *n += s.count,
                None => out.push((s.item.to_string(), s.count)),
            }
        }
        out
    }
    /// Takes up to `count` plain `item`, the main hand first; what it took.
    pub(super) fn native_take_items(&mut self, item: &str, count: u32) -> u32 {
        if self.player.is_none() {
            return 0;
        }
        let selected = self.selected;
        take_from(
            slots(&mut self.inventory, &mut self.offhand),
            selected,
            item,
            count,
        )
    }
    /// How many plain `item` still fit; 0 for an unknown item.
    pub(super) fn native_room_for(&mut self, item: &str) -> u32 {
        let Some(max) = self.res.items().get(item).map(|i| i.max_stack) else {
            return 0;
        };
        room_in(
            &slots(&mut self.inventory, &mut self.offhand),
            item,
            max.max(1),
        )
    }
    /// Gives up to `count` plain `item` where they fit, as `Inventory.add`
    /// places them; what it gave (nothing is dropped). 0 for an unknown item.
    pub(super) fn native_give_items(&mut self, item: &str, count: u32) -> u32 {
        let Some(max) = self.res.items().get(item).map(|i| i.max_stack) else {
            return 0;
        };
        if self.player.is_none() {
            return 0;
        }
        let selected = self.selected;
        add_to(
            slots(&mut self.inventory, &mut self.offhand),
            selected,
            item,
            max.max(1),
            count,
        )
    }
}

fn system_chat(line: &Text) -> C {
    C::SystemChat(play::SystemChat {
        content: component_nbt(&line.component()),
        overlay: false,
    })
}

/// Where the game calls the server hook points.
impl Game<'_> {
    /// Asks the mods for their commands once per hub, and puts them in every
    /// player's command tree.
    pub(super) fn register_native_commands(&mut self, host: &mut LanHost) {
        if host.native_commands.is_some() {
            return;
        }
        let mut commands = Vec::new();
        self.native_register_commands(&mut commands);
        let (tree, levels, problems) =
            command_tree(&commands, |name| host.hub.vanilla_command(name));
        for p in problems {
            log::warn!("Horizon: a native mod's command was left out: {p}");
        }
        host.hub.set_mod_commands(0, tree);
        host.native_commands = Some(levels);
    }

    /// The level a guest needs for a mod's command (`line`, without its
    /// slash), when a mod registered it.
    pub(super) fn native_command_level(host: &LanHost, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        host.native_commands.as_ref()?.get(name).copied()
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
        let Some(g) = host.hub.guest(conn) else {
            return false;
        };
        let player = NativeServerPlayer {
            uuid: g.uuid,
            name: g.name.clone(),
            permission,
            dimension: g.dimension.clone(),
            position: g.position,
            game_mode: g.game_mode,
            health: g.health,
        };
        let mut ran = false;
        self.native_perform_command(&player, line, &mut ran);
        if !ran {
            // Registered, but no mod took it: vanilla's unknown command.
            self.native_feedback(
                Text::translate("command.unknown.command", &[])
                    .with_color(horizon_client::text::RED),
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(commands: &[NativeCommand]) -> (Value, HashMap<String, u8>, Vec<String>) {
        command_tree(commands, |name| name == "give")
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
    fn items_are_taken_from_the_hand_first_and_only_plain() {
        let mut inv = vec![None; 36];
        inv[0] = stack("minecraft:diamond", 10);
        inv[4] = stack("minecraft:diamond", 5);
        inv[8] = stack("minecraft:diamond", 7);
        inv[8].as_mut().unwrap().damage = 3;
        let mut off = stack("minecraft:diamond", 2);
        assert_eq!(
            count_in(&slots(&mut inv, &mut off), "minecraft:diamond"),
            17
        );
        assert_eq!(
            take_from(slots(&mut inv, &mut off), 4, "minecraft:diamond", 6),
            6
        );
        assert!(inv[4].is_none(), "the selected slot first");
        assert_eq!(off.as_ref().unwrap().count, 1, "then the off hand");
        assert_eq!(
            take_from(slots(&mut inv, &mut off), 4, "minecraft:diamond", 100),
            11
        );
        assert_eq!(inv[8].as_ref().unwrap().count, 7, "a damaged stack stays");
        assert!(inv[0].is_none() && off.is_none());
    }

    #[test]
    fn items_are_given_only_where_they_fit() {
        let mut inv = vec![stack("minecraft:dirt", 64); 36];
        inv[3] = stack("minecraft:diamond", 60);
        inv[9] = None;
        let mut off = None;
        assert_eq!(
            room_in(&slots(&mut inv, &mut off), "minecraft:diamond", 64),
            4 + 64
        );
        assert_eq!(
            add_to(slots(&mut inv, &mut off), 0, "minecraft:diamond", 64, 200),
            68
        );
        assert_eq!(inv[3].as_ref().unwrap().count, 64, "onto its stack first");
        assert!(off.is_none(), "never into an empty off hand (getFreeSlot)");
        assert_eq!(inv[9].as_ref().unwrap().count, 64);
        assert_eq!(
            room_in(&slots(&mut inv, &mut off), "minecraft:diamond", 64),
            0
        );
        assert_eq!(
            add_to(slots(&mut inv, &mut off), 0, "minecraft:diamond", 64, 1),
            0
        );
    }

    #[test]
    fn usages_merge_into_one_tree_with_runnable_nodes() {
        let (t, levels, problems) = tree(&[
            NativeCommand::new("balance [player:player]"),
            NativeCommand::new("pay <player:player> <amount:integer>"),
            NativeCommand::op("eco give <player:player> <amount:integer>", 3),
            NativeCommand::op("eco take <player:player> <amount:integer>", 3),
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
    fn a_command_takes_its_lowest_level() {
        let (t, levels, _) = tree(&[
            NativeCommand::new("warp [name:word]"),
            NativeCommand::op("warp set <name:word>", 2),
        ]);
        assert_eq!(levels["warp"], 0);
        assert!(t["children"]["warp"].get("permissions").is_none());
        let set = &t["children"]["warp"]["children"]["set"];
        assert_eq!(set["permissions"]["permission"]["level"], "gamemasters");
    }

    #[test]
    fn bad_usages_and_vanilla_names_are_left_out() {
        let (t, levels, problems) = tree(&[
            NativeCommand::new("give <x:integer>"),
            NativeCommand::new("a [x:integer] <y:integer>"),
            NativeCommand::new("b <x:text> <y:word>"),
            NativeCommand::new("c <x:colour>"),
            NativeCommand::new("<x:integer>"),
            NativeCommand::new("Bad"),
            NativeCommand::new("d <x:integer>"),
            NativeCommand::new("d <x:word>"),
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
