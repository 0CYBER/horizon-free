//! Command execution (the `execute` side of each vanilla command class):
//! entity selectors resolved against the game, vanilla feedback and errors.
mod effects;
pub(super) mod execute;
mod text_commands;

use super::{Dim, Game, Screen, chat::PLAYER_NAME};
use horizon_client::{
    command::{
        Arg, CommandError, Context, Reader,
        args::{BlockInput, Coordinates, GameMode, ItemInput, Selector, Sort},
        vanilla::{self, DIFFICULTIES},
    },
    gamerules::{self, Value},
    items::ItemStack,
    text::{self, Style, Text},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::registry::BlockStateId;

type R<T> = Result<T, CommandError>;

/// An entity a selector found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Player,
    Mob(u32),
    Item(u32),
    Orb(usize),
    Arrow(usize),
}
fn err(key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
    CommandError::tr(key, args)
}
fn texts(args: &[Text]) -> Vec<Text> {
    args.to_vec()
}
/// Java's String.valueOf(float/double) for the values commands print.
fn java_float(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1.0e7 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}
/// String.format(Locale.ROOT, "%f").
fn format_double(v: f64) -> String {
    format!("{v:.6}")
}

impl Game<'_> {
    /// Commands.performCommand for a line without its slash.
    pub(super) fn run_command(&mut self, command: &str) {
        let d = vanilla::dispatcher();
        let parse = d.parse(command, &self.ui.registries);
        let result = match parse.executable() {
            Err(e) => {
                self.syntax_failure(&e, command);
                return;
            }
            Ok((ctx, id)) => {
                let ctx = ctx.clone();
                if parse
                    .context
                    .literals
                    .first()
                    .is_some_and(|w| w == "execute")
                {
                    let root = parse.context.clone();
                    self.run_execute(&root)
                } else {
                    self.execute_command(id, &ctx)
                }
            }
        };
        if let Err(e) = result {
            if e.input.is_some() {
                self.syntax_failure(&e, command);
            } else {
                self.command_failure(e.message);
            }
        }
    }
    fn syntax_failure(&mut self, e: &CommandError, command: &str) {
        self.command_failure(e.message.clone());
        let Some(input) = &e.input else { return };
        let cursor = e.cursor.min(input.len());
        let suggest = Style {
            suggest: Some(format!("/{command}")),
            ..Style::default()
        };
        let mut context = Text::empty();
        if cursor > 10 {
            context.push("...");
        }
        let start = (0..=cursor.saturating_sub(10))
            .rev()
            .find(|&i| input.is_char_boundary(i))
            .unwrap_or(0);
        context.push(&input[start..cursor]);
        if cursor < input.len() {
            context.push(
                Text::literal(&input[cursor..])
                    .with_color(text::RED)
                    .underlined(),
            );
        }
        context.push(
            Text::translate("command.context.here", &[])
                .with_color(text::RED)
                .italic(),
        );
        self.command_failure(context.with_color(text::GRAY).with_style(&suggest));
    }
    /// CommandSourceStack.sendFailure: red text.
    fn command_failure(&mut self, message: Text) {
        self.system_message(message.with_color(text::RED));
    }
    /// sendSuccess honouring `send_command_feedback`.
    fn feedback(&mut self, message: Text) {
        if self.ui.rules.bool("send_command_feedback") {
            self.system_message(message);
        }
    }
    fn source_position(&self) -> [f64; 3] {
        if let Some(s) = &self.command_source {
            return s.pos;
        }
        match &self.player {
            Some(p) => [p.position.x, p.position.y, p.position.z],
            None => self.spawn.map_or([0.0, 64.0, 0.0], |s| [s.x, s.y, s.z]),
        }
    }
    fn source_rotation(&self) -> [f32; 2] {
        self.command_source
            .map_or([self.yaw, self.pitch], |s| s.rot)
    }
    /// LocalCoordinates use the anchored position; world coordinates the position.
    fn resolve_coords(&self, c: &Coordinates) -> [f64; 3] {
        let origin = match (c, &self.command_source) {
            (Coordinates::Local(_), Some(s)) => self.anchor_position(s),
            _ => self.source_position(),
        };
        c.position(origin, self.source_rotation())
    }
    fn coords(&self, ctx: &Context, name: &str) -> Option<Coordinates> {
        match ctx.get(name) {
            Some(Arg::Coords(c)) => Some(c.clone()),
            _ => None,
        }
    }
    fn position_arg(&self, ctx: &Context, name: &str) -> Option<[f64; 3]> {
        self.coords(ctx, name).map(|c| self.resolve_coords(&c))
    }
    /// BlockPosArgument.getLoadedBlockPos.
    fn block_pos_arg(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos.missing.int", &[]))?;
        // BlockPos.containing(getPosition)
        let p = self.resolve_coords(&c).map(|v| v.floor() as i32);
        if !self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
            return Err(err("argument.pos.unloaded", &[]));
        }
        if p[1] < horizon_client::chunk::MIN_Y
            || p[1] >= horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT
        {
            return Err(err("argument.pos.outofworld", &[]));
        }
        Ok(p)
    }
    fn int(ctx: &Context, name: &str) -> Option<i32> {
        match ctx.get(name) {
            Some(Arg::Int(v)) => Some(*v),
            _ => None,
        }
    }
    // ---- entities -------------------------------------------------------

    fn target_position(&self, t: Target) -> Option<[f64; 3]> {
        match t {
            Target::Player => self
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| [m.position.x, m.position.y, m.position.z]),
            Target::Item(id) => self
                .dropped
                .iter()
                .find(|i| i.id == id)
                .map(|i| [i.position.x, i.position.y, i.position.z]),
            Target::Orb(i) => self
                .orbs
                .get(i)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Arrow(i) => self
                .arrows
                .get(i)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
        }
    }
    fn target_type(&self, t: Target) -> String {
        match t {
            Target::Player => "minecraft:player".into(),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| {
                    horizon_client::command::args::normalize(&m.kind.id)
                }),
            Target::Item(_) => "minecraft:item".into(),
            Target::Orb(_) => "minecraft:experience_orb".into(),
            Target::Arrow(_) => "minecraft:arrow".into(),
        }
    }
    /// Entity.getDisplayName.
    pub(super) fn target_name(&self, t: Target) -> Text {
        match t {
            Target::Player => Text::literal(PLAYER_NAME),
            Target::Item(id) => {
                let item = self
                    .dropped
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.stack.item.to_string())
                    .unwrap_or_default();
                Text::literal(self.res.names.item(&item))
            }
            other => {
                let ty = self.target_type(other);
                Text::translate(&format!("entity.{}", ty.replace(':', ".")), &[])
            }
        }
    }
    /// EntitySelector.findEntities.
    fn select(&mut self, s: &Selector) -> Vec<Target> {
        if let Some(name) = &s.player_name {
            return if name.eq_ignore_ascii_case(PLAYER_NAME) && self.player.is_some() {
                vec![Target::Player]
            } else {
                Vec::new()
            };
        }
        let origin = {
            let p = self.source_position();
            [
                s.pos[0].unwrap_or(p[0]),
                s.pos[1].unwrap_or(p[1]),
                s.pos[2].unwrap_or(p[2]),
            ]
        };
        let mut candidates: Vec<Target> = Vec::new();
        match (s.kind, self.command_source) {
            // @s: the executing entity (none for a server source).
            ('s', Some(source)) => candidates.extend(source.entity),
            _ if self.player.is_some() => candidates.push(Target::Player),
            _ => {}
        }
        if s.kind != 's' {
            candidates.extend(
                self.mobs
                    .iter()
                    .filter(|m| m.death_time == 0)
                    .map(|m| Target::Mob(m.id)),
            );
            candidates.extend(self.dropped.iter().map(|i| Target::Item(i.id)));
            candidates.extend((0..self.orbs.len()).map(Target::Orb));
            candidates.extend((0..self.arrows.len()).map(Target::Arrow));
        }
        let level = self.experience.level;
        let creative = self.creative;
        let mut out: Vec<(Target, f64)> = candidates
            .into_iter()
            .filter_map(|t| {
                let p = self.target_position(t)?;
                let ty = self.target_type(t);
                // type=: every positive must match, no negative may match.
                for (want, neg) in &s.types {
                    let matches = if let Some(tag) = want.strip_prefix('#') {
                        self.entity_tag_matches(tag, &ty)
                    } else {
                        *want == ty
                    };
                    if matches == *neg {
                        return None;
                    }
                }
                if !s.include_entities
                    && t != Target::Player
                    && s.types.iter().all(|(w, n)| *n || w != &ty)
                    && s.kind != 'e'
                    && s.kind != 'n'
                {
                    return None;
                }
                for (name, neg) in &s.names {
                    let n = self.target_name(t).plain();
                    if (n == *name) == *neg {
                        return None;
                    }
                }
                for (_, neg) in &s.tags {
                    if !*neg {
                        return None;
                    }
                }
                if let Some((lo, hi)) = s.level
                    && (t != Target::Player
                        || lo.is_some_and(|l| level < l)
                        || hi.is_some_and(|h| level > h))
                {
                    return None;
                }
                for (mode, neg) in &s.gamemodes {
                    let current = if creative { "creative" } else { "survival" };
                    if t != Target::Player || (current == mode) == *neg {
                        return None;
                    }
                }
                let d2 = (p[0] - origin[0]).powi(2)
                    + (p[1] - origin[1]).powi(2)
                    + (p[2] - origin[2]).powi(2);
                if let Some((lo, hi)) = s.distance
                    && (lo.is_some_and(|l| d2 < l * l) || hi.is_some_and(|h| d2 > h * h))
                {
                    return None;
                }
                if s.delta.iter().any(Option::is_some) {
                    let d = s.delta.map(|v| v.unwrap_or(0.0));
                    for i in 0..3 {
                        let (a, b) = (
                            origin[i].min(origin[i] + d[i]),
                            origin[i].max(origin[i] + d[i]) + 1.0,
                        );
                        if p[i] < a || p[i] >= b {
                            return None;
                        }
                    }
                }
                Some((t, d2))
            })
            .collect();
        match s.sort {
            Sort::Nearest => out.sort_by(|a, b| a.1.total_cmp(&b.1)),
            Sort::Furthest => out.sort_by(|a, b| b.1.total_cmp(&a.1)),
            Sort::Random => {
                for i in (1..out.len()).rev() {
                    let j = self.world.random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                    out.swap(i, j);
                }
            }
            Sort::Arbitrary => {}
        }
        out.into_iter()
            .take(s.limit.max(1))
            .map(|(t, _)| t)
            .collect()
    }
    fn entity_tag_matches(&self, tag: &str, ty: &str) -> bool {
        let path = self.options.resources.join(format!(
            "data/minecraft/tags/entity_type/{}.json",
            tag.trim_start_matches("minecraft:")
        ));
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap_or_default())
            .ok()
            .is_some_and(|v| {
                v["values"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|e| e.as_str() == Some(ty)))
            })
    }
    /// EntityArgument.getEntities / getPlayers / getEntity / getPlayer.
    fn entities(&mut self, ctx: &Context, name: &str, players: bool) -> R<Vec<Target>> {
        let Some(Arg::Entity(s)) = ctx.get(name).cloned() else {
            return Ok(vec![Target::Player]);
        };
        let found: Vec<Target> = self
            .select(&s)
            .into_iter()
            .filter(|t| !players || *t == Target::Player)
            .collect();
        if found.is_empty() {
            return Err(err(
                if players {
                    "argument.entity.notfound.player"
                } else {
                    "argument.entity.notfound.entity"
                },
                &[],
            ));
        }
        Ok(found)
    }
    // ---- dispatch -------------------------------------------------------

    fn execute_command(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "help" => self.cmd_help(ctx),
            "gamemode" => self.cmd_gamemode(ctx),
            "defaultgamemode" => {
                let Some(Arg::GameMode(m)) = ctx.get("gamemode") else {
                    return Ok(0);
                };
                self.feedback(Text::translate(
                    "commands.defaultgamemode.success",
                    &[Text::translate(&format!("gameMode.{}", m.name()), &[])],
                ));
                Ok(0)
            }
            "give" => self.cmd_give(ctx),
            "teleport.location"
            | "teleport.destination"
            | "teleport.targets.location"
            | "teleport.targets.destination" => self.cmd_teleport(id, ctx),
            id if id.starts_with("time.") => self.cmd_time(id, ctx),
            "weather" => self.cmd_weather(ctx),
            "kill" => self.cmd_kill(ctx),
            "summon" => self.cmd_summon(ctx),
            "effect.give" => self.cmd_effect_give(ctx),
            "effect.clear" => self.cmd_effect_clear(ctx),
            "enchant" => self.cmd_enchant(ctx),
            "tellraw" => self.cmd_tellraw(ctx),
            "title" => self.cmd_title(ctx),
            "setblock" => self.cmd_setblock(ctx),
            "fill" => self.cmd_fill(ctx),
            "clone" => self.cmd_clone(ctx),
            "experience" => self.cmd_experience(ctx),
            "advancement" => self.cmd_advancement(ctx),
            "gamerule" => self.cmd_gamerule(ctx),
            "difficulty" => self.cmd_difficulty(ctx),
            "seed" => {
                let seed = self.level.seed.to_string();
                let green = Style {
                    color: Some(text::GREEN),
                    suggest: None,
                    ..Style::default()
                };
                let value = Text::literal("[")
                    .append(Text::styled(seed.clone(), green))
                    .append("]");
                self.system_message(Text::translate("commands.seed.success", &[value]));
                Ok(self.level.seed as i32)
            }
            "spawnpoint" => self.cmd_spawnpoint(ctx),
            "setworldspawn" => self.cmd_setworldspawn(ctx),
            "clear" => self.cmd_clear(ctx),
            "locate.structure" | "locate.biome" => self.cmd_locate(id, ctx),
            "warden_spawn_tracker.clear" | "warden_spawn_tracker.set" => {
                let level = Self::int(ctx, "warning_level");
                self.cmd_warden_spawn_tracker(level);
                Ok(1)
            }
            "say" | "me" | "msg" => self.cmd_say(id, ctx),
            "list" => {
                let names = if ctx.has("uuids") {
                    Text::translate(
                        "commands.list.nameAndId",
                        &[
                            Text::literal(PLAYER_NAME),
                            Text::literal("00000000-0000-0000-0000-000000000000"),
                        ],
                    )
                } else {
                    Text::literal(PLAYER_NAME)
                };
                self.system_message(Text::translate(
                    "commands.list.players",
                    &[Text::literal("1"), Text::literal("8"), names],
                ));
                Ok(1)
            }
            _ => Err(err("command.failed", &[])),
        }
    }
    fn cmd_help(&mut self, ctx: &Context) -> R<i32> {
        let d = vanilla::dispatcher();
        let level = self.ui.registries.permission;
        match ctx.get("command") {
            None => {
                let usage = d.smart_usage(0, level);
                let n = usage.len() as i32;
                for (_, line) in usage {
                    self.system_message(Text::literal(format!("/{line}")));
                }
                Ok(n)
            }
            Some(Arg::Str(command)) => {
                let parse = d.parse(command, &self.ui.registries);
                if parse.context.nodes.is_empty() {
                    return Err(err("commands.help.failed", &[]));
                }
                let last = parse.context.nodes.last().map(|n| n.0).unwrap_or(0);
                let usage = d.smart_usage(last, level);
                let prefix = &command[..parse.context.range.1.min(command.len())];
                for (_, line) in &usage {
                    self.system_message(Text::literal(format!("/{prefix} {line}")));
                }
                if usage.is_empty() {
                    self.system_message(Text::literal(format!("/{prefix}")));
                }
                Ok(usage.len().max(1) as i32)
            }
            _ => Ok(0),
        }
    }
    fn set_game_mode(&mut self, mode: GameMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.set_mode(mode);
        if self.screen == Screen::Inventory && (self.creative || self.spectator()) {
            self.screen = Screen::Playing;
        }
        true
    }
    fn cmd_gamemode(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::GameMode(mode)) = ctx.get("gamemode").cloned() else {
            return Ok(0);
        };
        let targets = if ctx.get("target").is_some() {
            self.entities(ctx, "target", true)?
        } else {
            vec![Target::Player]
        };
        let mut count = 0;
        for t in targets {
            if t != Target::Player || !self.set_game_mode(mode) {
                continue;
            }
            count += 1;
            let mode_name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            if ctx.get("target").is_none() {
                self.feedback(Text::translate(
                    "commands.gamemode.success.self",
                    &[mode_name],
                ));
            } else {
                self.system_message(Text::translate(
                    "gameMode.changed",
                    std::slice::from_ref(&mode_name),
                ));
                self.feedback(Text::translate(
                    "commands.gamemode.success.other",
                    &[Text::literal(PLAYER_NAME), mode_name],
                ));
            }
        }
        Ok(count)
    }
    /// ItemStack.getDisplayName: "[Name]" in the rarity colour.
    pub(super) fn item_display_name(&self, item: &str) -> Text {
        let rarity = self.item_rarity(item);
        let color = match rarity.as_str() {
            "uncommon" => text::YELLOW,
            "rare" => text::AQUA,
            "epic" => text::LIGHT_PURPLE,
            _ => text::WHITE,
        };
        Text::literal("[")
            .append(Text::literal(self.res.names.item(item)))
            .append("]")
            .with_color(color)
    }
    fn item_rarity(&self, item: &str) -> String {
        static RARITY: std::sync::OnceLock<std::collections::HashMap<String, String>> =
            std::sync::OnceLock::new();
        let map = RARITY.get_or_init(|| {
            let v: serde_json::Value = serde_json::from_slice(
                &std::fs::read(self.options.resources.join("items.json")).unwrap_or_default(),
            )
            .unwrap_or_default();
            v.as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, c)| {
                            c["minecraft:rarity"]
                                .as_str()
                                .map(|r| (k.clone(), r.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        map.get(item).cloned().unwrap_or_else(|| "common".into())
    }
    fn cmd_give(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Item(ItemInput { id, components, .. })) = ctx.get("item").cloned() else {
            return Ok(0);
        };
        let (damage, extra) = match components
            .as_deref()
            .map(horizon_client::item_data::command_components)
        {
            Some(Ok(tag)) => horizon_client::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let count = Self::int(ctx, "count").unwrap_or(1);
        let targets = self.entities(ctx, "targets", true)?;
        let max_stack = self.res.items.get(&id).map_or(64, |i| i.max_stack) as i32;
        if count > max_stack * 100 {
            return Err(CommandError::new(Text::translate(
                "commands.give.failed.toomanyitems",
                &[
                    Text::literal((max_stack * 100).to_string()),
                    self.item_display_name(&id),
                ],
            )));
        }
        for _ in &targets {
            let mut remaining = count;
            while remaining > 0 {
                let size = remaining.min(max_stack);
                remaining -= size;
                let left = self.give(ItemStack {
                    item: id.as_str().into(),
                    count: size as u32,
                    damage,
                    extra: extra.clone(),
                });
                if let Some(rest) = left {
                    // Inventory full: drop the rest at the player's feet without pickup delay.
                    let p = self.source_position();
                    self.spawn_item(rest, [p[0], p[1] + 1.32, p[2]], [0.0, 0.0, 0.0], 0);
                } else {
                    let r = &mut self.world.random;
                    let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
                    let p = self.source_position();
                    self.audio
                        .play("minecraft:entity.item.pickup", Some(p), 0.2, pitch);
                }
            }
        }
        let name = self.item_display_name(&id);
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.give.success.single",
                &[
                    Text::literal(count.to_string()),
                    name,
                    Text::literal(PLAYER_NAME),
                ],
            )
        } else {
            Text::translate(
                "commands.give.success.multiple",
                &texts(&[
                    Text::literal(count.to_string()),
                    name,
                    Text::literal(targets.len().to_string()),
                ]),
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    fn teleport_to(&mut self, t: Target, pos: [f64; 3], rotation: Option<[f32; 2]>) {
        let v = Vec3::new(pos[0], pos[1], pos[2]);
        match t {
            Target::Player => {
                if let Some(p) = self.player.as_mut() {
                    p.position = v;
                    p.velocity = Vec3::new(0.0, 0.0, 0.0);
                    p.fall_distance = 0.0;
                    p.ground.on_ground = false;
                }
                self.previous_position = v;
                if let Some([yaw, pitch]) = rotation {
                    self.yaw = yaw;
                    self.pitch = pitch.clamp(-90.0, 90.0);
                }
            }
            Target::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.position = v;
                    m.previous = v;
                    if let Some([yaw, _]) = rotation {
                        m.yaw = yaw;
                        m.body_yaw = yaw;
                    }
                }
            }
            Target::Item(id) => {
                if let Some(i) = self.dropped.iter_mut().find(|i| i.id == id) {
                    i.position = v;
                    i.previous = v;
                }
            }
            Target::Orb(i) => {
                if let Some(o) = self.orbs.get_mut(i) {
                    o.position = v;
                    o.previous = v;
                }
            }
            Target::Arrow(i) => {
                if let Some(a) = self.arrows.get_mut(i) {
                    a.position = v;
                    a.previous = v;
                }
            }
        }
    }
    fn cmd_teleport(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = if id.starts_with("teleport.targets") {
            self.entities(ctx, "targets", false)?
        } else {
            vec![Target::Player]
        };
        if id.ends_with("destination") {
            let dest = self.entities(ctx, "destination", false)?[0];
            let pos = self.target_position(dest).unwrap_or(self.source_position());
            for &t in &targets {
                self.teleport_to(t, pos, None);
            }
            let message = if targets.len() == 1 {
                Text::translate(
                    "commands.teleport.success.entity.single",
                    &[self.target_name(targets[0]), self.target_name(dest)],
                )
            } else {
                Text::translate(
                    "commands.teleport.success.entity.multiple",
                    &[
                        Text::literal(targets.len().to_string()),
                        self.target_name(dest),
                    ],
                )
            };
            self.feedback(message);
            return Ok(targets.len() as i32);
        }
        let pos = self
            .position_arg(ctx, "location")
            .unwrap_or(self.source_position());
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
            || !(-20_000_000.0..20_000_000.0).contains(&pos[1])
        {
            return Err(err("commands.teleport.invalidPosition", &[]));
        }
        let mut rotation = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => Some(c.rotation(self.source_rotation())),
            _ => None,
        };
        // facing: look from the new position at a location or entity.
        let facing = self.position_arg(ctx, "facingLocation").or_else(|| {
            let Some(Arg::Entity(_)) = ctx.get("facingEntity") else {
                return None;
            };
            None
        });
        let facing = match (facing, ctx.get("facingEntity")) {
            (Some(f), _) => Some(f),
            (None, Some(_)) => {
                let e = self.entities(ctx, "facingEntity", false)?[0];
                let eyes = matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true)));
                self.target_position(e)
                    .map(|p| [p[0], p[1] + if eyes { 1.62 } else { 0.0 }, p[2]])
            }
            _ => None,
        };
        if let Some(f) = facing {
            let eye = pos[1] + 1.62;
            let (dx, dy, dz) = (f[0] - pos[0], f[1] - eye, f[2] - pos[2]);
            let horizontal = (dx * dx + dz * dz).sqrt();
            rotation = Some([
                ((dz.atan2(dx).to_degrees()) - 90.0) as f32,
                (-(dy.atan2(horizontal).to_degrees())) as f32,
            ]);
        }
        for &t in &targets {
            self.teleport_to(t, pos, rotation);
        }
        let [x, y, z] = pos.map(|v| Text::literal(format_double(v)));
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.teleport.success.location.single",
                &[self.target_name(targets[0]), x, y, z],
            )
        } else {
            Text::translate(
                "commands.teleport.success.location.multiple",
                &[Text::literal(targets.len().to_string()), x, y, z],
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    fn cmd_time(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "time.query.gametime" {
            let t = self.world.game_time;
            self.system_message(Text::tr("commands.time.query.gametime", &[&t]));
            return Ok((t % i64::from(i32::MAX)) as i32);
        }
        // `time of <clock>` names the clock; otherwise the dimension's default clock.
        let clock = match ctx.get("clock") {
            Some(Arg::Resource(c)) => c.clone(),
            _ => {
                if self.dim == Dim::Nether {
                    return Err(err(
                        "commands.time.no_default_clock",
                        &[&"minecraft:the_nether"],
                    ));
                }
                "minecraft:overworld".to_owned()
            }
        };
        if clock != "minecraft:overworld" {
            // Only the overworld clock runs in Horizon (no End yet).
            return Err(err("commands.time.no_default_clock", &[&clock]));
        }
        let total = self.world.day_time;
        match id {
            "time.set" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                if total == t {
                    return Err(err("commands.time.set.already_at_time", &[&clock, &t]));
                }
                self.world.day_time = t;
                self.ui.clock.partial = 0.0;
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &t]));
                Ok(t as i32)
            }
            "time.set.marker" => {
                let Some(Arg::Resource(marker)) = ctx.get("timemarker").cloned() else {
                    return Ok(0);
                };
                let Some(m) = self.ui.markers.get(&marker).copied() else {
                    return Err(err(
                        "commands.time.no_time_marker_found",
                        &[&marker, &clock],
                    ));
                };
                if m.occurs_at(total) {
                    return Err(err(
                        "commands.time.set.already_at_time_marker",
                        &[&clock, &marker],
                    ));
                }
                self.world.day_time = m.resolve(total);
                self.ui.clock.partial = 0.0;
                self.feedback(Text::tr(
                    "commands.time.set.time_marker",
                    &[&clock, &marker],
                ));
                Ok((self.world.day_time % i64::from(i32::MAX)) as i32)
            }
            "time.add" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                self.world.day_time = (total + t).max(0);
                let now = self.world.day_time;
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &now]));
                Ok((now % i64::from(i32::MAX)) as i32)
            }
            "time.pause" | "time.resume" => {
                let pause = id == "time.pause";
                if self.ui.clock.paused == pause {
                    return Err(err(
                        if pause {
                            "commands.time.pause.already_paused"
                        } else {
                            "commands.time.pause.already_running"
                        },
                        &[&clock],
                    ));
                }
                self.ui.clock.paused = pause;
                self.feedback(Text::tr(
                    if pause {
                        "commands.time.pause"
                    } else {
                        "commands.time.resume"
                    },
                    &[&clock],
                ));
                Ok(1)
            }
            "time.rate" => {
                let rate = match ctx.get("rate") {
                    Some(Arg::Float(r)) => *r,
                    _ => 1.0,
                };
                if self.ui.clock.rate == rate {
                    return Err(err(
                        "commands.time.rate.already_same",
                        &[&clock, &java_float(f64::from(rate))],
                    ));
                }
                self.ui.clock.rate = rate;
                self.feedback(Text::tr(
                    "commands.time.rate",
                    &[&clock, &java_float(f64::from(rate))],
                ));
                Ok(1)
            }
            "time.query.time" => {
                self.system_message(Text::tr("commands.time.query.absolute", &[&clock, &total]));
                Ok((total % i64::from(i32::MAX)) as i32)
            }
            "time.query.timeline" | "time.query.repetition" => {
                let Some(Arg::Resource(timeline)) = ctx.get("timeline").cloned() else {
                    return Ok(0);
                };
                let Some(period) = self.ui.timelines.get(&timeline).copied() else {
                    return Err(err(
                        "commands.time.wrong_timeline_for_clock",
                        &[&clock, &timeline],
                    ));
                };
                if id == "time.query.timeline" {
                    let ticks = period.map_or(total, |p| total.rem_euclid(p));
                    self.system_message(Text::tr(
                        "commands.time.query.timeline",
                        &[&timeline, &ticks],
                    ));
                    Ok(ticks as i32)
                } else {
                    let reps = period.map_or(0, |p| total / p);
                    self.system_message(Text::tr(
                        "commands.time.query.timeline.repetitions",
                        &[&timeline, &reps],
                    ));
                    Ok(reps as i32)
                }
            }
            _ => Ok(0),
        }
    }
    fn cmd_weather(&mut self, ctx: &Context) -> R<i32> {
        let duration = Self::int(ctx, "duration");
        let sample = |min: i32, max: i32, r: &mut horizon_core::random::LegacyRandom| {
            min + r.next_int_bound(max - min + 1).unwrap_or(0)
        };
        let w = &mut self.world.weather;
        let r = &mut self.world.random;
        let (key, clear, time, raining, thundering) = if ctx.has("clear") {
            (
                "commands.weather.set.clear",
                duration.unwrap_or_else(|| sample(12_000, 180_000, r)),
                0,
                false,
                false,
            )
        } else if ctx.has("rain") {
            (
                "commands.weather.set.rain",
                0,
                duration.unwrap_or_else(|| sample(12_000, 24_000, r)),
                true,
                false,
            )
        } else {
            (
                "commands.weather.set.thunder",
                0,
                duration.unwrap_or_else(|| sample(3_600, 15_600, r)),
                true,
                true,
            )
        };
        // ServerLevel.setWeatherParameters.
        w.clear_time = clear;
        w.rain_time = time;
        w.thunder_time = time;
        w.raining = raining;
        w.thundering = thundering;
        self.feedback(Text::translate(key, &[]));
        Ok(duration.unwrap_or(0))
    }
    fn kill_target(&mut self, t: Target) {
        match t {
            Target::Player => {
                if let Some(p) = self.player.as_mut() {
                    let _ = p.set_health(0.0, false);
                }
                self.ui.death_cause = Some("death.attack.genericKill".into());
            }
            Target::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.health = 0.0;
                    if m.death_time == 0 {
                        m.death_time = 1;
                    }
                }
            }
            Target::Item(id) => self.dropped.retain(|i| i.id != id),
            Target::Orb(i) => {
                if i < self.orbs.len() {
                    self.orbs[i].age = u32::MAX / 2;
                }
            }
            Target::Arrow(i) => {
                if i < self.arrows.len() {
                    self.arrows[i].age = u32::MAX / 2;
                }
            }
        }
    }
    fn cmd_kill(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", false)?
        } else {
            vec![Target::Player]
        };
        let names: Vec<Text> = targets.iter().map(|&t| self.target_name(t)).collect();
        // Remove indexed entities from the back so earlier indices stay valid.
        let mut ordered = targets.clone();
        ordered.sort_by_key(|t| {
            std::cmp::Reverse(match t {
                Target::Orb(i) | Target::Arrow(i) => *i,
                _ => 0,
            })
        });
        for t in ordered {
            self.kill_target(t);
        }
        self.orbs.retain(|o| o.age < u32::MAX / 2);
        self.arrows.retain(|a| a.age < u32::MAX / 2);
        let message = if targets.len() == 1 {
            Text::translate("commands.kill.success.single", &[names[0].clone()])
        } else {
            Text::tr("commands.kill.success.multiple", &[&targets.len()])
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    fn cmd_summon(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(kind)) = ctx.get("entity").cloned() else {
            return Ok(0);
        };
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or(self.source_position());
        let block_y = pos[1].floor() as i32;
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-20_000_000..20_000_000).contains(&block_y)
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
        {
            return Err(err("commands.summon.invalidPosition", &[]));
        }
        let short = kind.trim_start_matches("minecraft:").to_owned();
        let name;
        if let Some(t) = self
            .res
            .mob_types
            .get(&format!("minecraft:{short}"))
            .cloned()
        {
            if self.ui.difficulty == 0 && t.category == "monster" {
                return Err(err("commands.summon.failed.peaceful", &[]));
            }
            self.spawn_mob(&short, pos, true);
            name = Text::translate(&format!("entity.minecraft.{short}"), &[]);
        } else {
            match short.as_str() {
                "item" => {
                    // An item entity needs an item; vanilla spawns an empty one that despawns.
                    return Err(err("commands.summon.failed", &[]));
                }
                "experience_orb" => {
                    self.orbs.push(horizon_client::entities::ExperienceOrb {
                        position: Vec3::new(pos[0], pos[1], pos[2]),
                        previous: Vec3::new(pos[0], pos[1], pos[2]),
                        velocity: Vec3::new(0.0, 0.0, 0.0),
                        value: 0,
                        age: 0,
                        on_ground: false,
                    });
                    name = Text::translate("entity.minecraft.experience_orb", &[]);
                }
                "lightning_bolt" => {
                    self.spawn_bolt(pos, false);
                    name = Text::translate("entity.minecraft.lightning_bolt", &[]);
                }
                k if super::minecarts::CartKind::from_id(&kind).is_some() => {
                    let cart_kind = super::minecarts::CartKind::from_id(&kind)
                        .unwrap_or(super::minecarts::CartKind::Rideable);
                    self.next_entity += 1;
                    let cart = super::minecarts::Minecart::new(
                        self.next_entity,
                        cart_kind,
                        Vec3::new(pos[0], pos[1], pos[2]),
                    );
                    self.carts.push(cart);
                    name = Text::translate(&format!("entity.minecraft.{k}"), &[]);
                }
                _ => return Err(err("commands.summon.failed", &[])),
            }
        }
        self.feedback(Text::translate("commands.summon.success", &[name]));
        Ok(1)
    }
    /// BlockStateArgument -> registry state.
    fn block_state(&self, b: &BlockInput) -> R<BlockStateId> {
        let state = horizon_world::chunk::BlockState {
            name: b.id.clone(),
            properties: b.properties.iter().cloned().collect(),
        };
        self.res
            .assets
            .registry
            .resolve(&state)
            .map_err(|_| err("argument.block.id.invalid", &[&b.id]))
    }
    /// BlockPredicateArgument test.
    fn block_matches(&self, b: &BlockInput, state: BlockStateId) -> bool {
        let Some(s) = self.res.assets.registry.state(state) else {
            return false;
        };
        let name_ok = if b.tag {
            let tag = b.id.trim_start_matches("minecraft:");
            self.res
                .items
                .block_tags
                .get(tag)
                .or_else(|| self.res.items.block_tags.get(&b.id))
                .is_some_and(|set| {
                    set.contains(&s.name) || set.contains(s.name.trim_start_matches("minecraft:"))
                })
        } else {
            s.name == b.id
        };
        name_ok
            && b.properties
                .iter()
                .all(|(k, v)| s.properties.get(k) == Some(v))
    }
    /// Place a state like BlockInput.place: false when nothing changed.
    fn place_state(&mut self, pos: [i32; 3], state: BlockStateId, strict: bool) -> R<bool> {
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        if previous == state {
            return Ok(false);
        }
        // A different block replaces the block entity (Level.setBlock).
        let (old, new) = (
            self.res.facts.get(previous).name.clone(),
            self.res.facts.get(state).name.clone(),
        );
        if old != new {
            let key = (pos[0], pos[1], pos[2]);
            self.jukebox_removed(pos);
            self.world.block_entities.remove(&key);
            if let Some(entity) = horizon_client::containers::BlockEntity::for_block(&new) {
                self.world.block_entities.insert(key, entity);
            }
        }
        if strict {
            self.with_level(|b, level, ticks| {
                b.set_block(
                    level,
                    ticks,
                    horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                    state,
                    horizon_world::level::flags::UPDATE_CLIENTS
                        | horizon_world::level::flags::UPDATE_SKIP_ALL_SIDEEFFECTS,
                );
            });
            self.process_level_events()
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        } else {
            self.level_set(pos, state)
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        }
        Ok(true)
    }
    fn cmd_setblock(&mut self, ctx: &Context) -> R<i32> {
        let pos = self.block_pos_arg(ctx, "pos")?;
        let Some(Arg::Block(b)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let state = self.block_state(&b)?;
        let current = self.world.block(pos[0], pos[1], pos[2]);
        if ctx.has("keep") && current != self.world.air {
            return Err(err("commands.setblock.failed", &[]));
        }
        if ctx.has("destroy") && current != self.world.air {
            let creative = self.creative;
            self.creative = false;
            let _ = self.break_block(pos);
            self.creative = creative;
        }
        if !self.place_state(pos, state, ctx.has("strict"))? {
            return Err(err("commands.setblock.failed", &[]));
        }
        self.feedback(Text::tr(
            "commands.setblock.success",
            &[&pos[0], &pos[1], &pos[2]],
        ));
        Ok(1)
    }
    fn cmd_fill(&mut self, ctx: &Context) -> R<i32> {
        let a = self.block_pos_arg(ctx, "from")?;
        let b = self.block_pos_arg(ctx, "to")?;
        let Some(Arg::Block(block)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let area = (0..3)
            .map(|i| i64::from(max[i] - min[i] + 1))
            .product::<i64>();
        let limit = self.ui.rules.int("max_block_modifications");
        if area > i64::from(limit) {
            return Err(err("commands.fill.toobig", &[&limit, &area]));
        }
        let state = self.block_state(&block)?;
        let air = self.world.air;
        let (hollow, outline, destroy, strict, keep) = (
            ctx.has("hollow"),
            ctx.has("outline"),
            ctx.has("destroy"),
            ctx.has("strict"),
            ctx.has("keep"),
        );
        let mut count = 0;
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let current = self.world.block(x, y, z);
                    if keep && current != air {
                        continue;
                    }
                    if let Some(f) = &filter
                        && !self.block_matches(f, current)
                    {
                        continue;
                    }
                    let edge = x == min[0]
                        || x == max[0]
                        || y == min[1]
                        || y == max[1]
                        || z == min[2]
                        || z == max[2];
                    let mut affected = false;
                    if destroy && current != air {
                        let creative = self.creative;
                        self.creative = false;
                        let _ = self.break_block([x, y, z]);
                        self.creative = creative;
                        affected = true;
                    }
                    let target = if (outline || hollow) && !edge {
                        if outline {
                            if affected {
                                count += 1;
                            }
                            continue;
                        }
                        air
                    } else {
                        state
                    };
                    if self.place_state([x, y, z], target, strict)? || affected {
                        count += 1;
                    }
                }
            }
        }
        if count == 0 {
            return Err(err("commands.fill.failed", &[]));
        }
        self.feedback(Text::tr("commands.fill.success", &[&count]));
        Ok(count)
    }
    fn cmd_clone(&mut self, ctx: &Context) -> R<i32> {
        let a = self.block_pos_arg(ctx, "begin")?;
        let b = self.block_pos_arg(ctx, "end")?;
        let dest = self.block_pos_arg(ctx, "destination")?;
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let dmax = [dest[0] + size[0], dest[1] + size[1], dest[2] + size[2]];
        let overlap = (0..3).all(|i| dest[i] <= max[i] && dmax[i] >= min[i]);
        let moving = ctx.has("move");
        if !ctx.has("force") && overlap {
            return Err(err("commands.clone.overlap", &[]));
        }
        let area = (0..3).map(|i| i64::from(size[i] + 1)).product::<i64>();
        let limit = self.ui.rules.int("max_block_modifications");
        if area > i64::from(limit) {
            return Err(err("commands.clone.toobig", &[&limit, &area]));
        }
        for p in [dest, dmax] {
            if !self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
                return Err(err("argument.pos.unloaded", &[]));
            }
        }
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let air = self.world.air;
        let masked = ctx.has("masked");
        let mut copies = Vec::new();
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let s = self.world.block(x, y, z);
                    if masked && s == air {
                        continue;
                    }
                    if let Some(f) = &filter
                        && !self.block_matches(f, s)
                    {
                        continue;
                    }
                    copies.push((
                        [x, y, z],
                        [
                            dest[0] + x - min[0],
                            dest[1] + y - min[1],
                            dest[2] + z - min[2],
                        ],
                        s,
                    ));
                }
            }
        }
        if moving {
            for (from, _, _) in &copies {
                self.place_state(*from, air, true)?;
            }
        }
        let strict = ctx.has("strict");
        let mut count = 0;
        for (_, to, s) in copies {
            if self.place_state(to, s, strict)? {
                count += 1;
            }
        }
        if count == 0 {
            return Err(err("commands.clone.failed", &[]));
        }
        self.feedback(Text::tr("commands.clone.success", &[&count]));
        Ok(count)
    }
    /// Player.giveExperiencePoints including negative amounts.
    fn give_points(&mut self, amount: i32) {
        let e = &mut self.experience;
        e.progress += amount as f32 / horizon_client::entities::Experience::needed(e.level) as f32;
        e.total = e.total.saturating_add(amount).clamp(0, i32::MAX);
        while e.progress < 0.0 {
            let f = e.progress * horizon_client::entities::Experience::needed(e.level) as f32;
            if e.level > 0 {
                e.level -= 1;
                e.progress = 1.0 + f / horizon_client::entities::Experience::needed(e.level) as f32;
            } else {
                e.level = 0;
                e.progress = 0.0;
            }
        }
        while e.progress >= 1.0 {
            e.progress =
                (e.progress - 1.0) * horizon_client::entities::Experience::needed(e.level) as f32;
            e.level += 1;
            e.progress /= horizon_client::entities::Experience::needed(e.level) as f32;
        }
    }
    /// AdvancementCommands.perform / performCriterion.
    fn cmd_advancement(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let grant = ctx.has("grant");
        let key = if grant { "grant" } else { "revoke" };
        let index = match ctx.get("advancement") {
            Some(Arg::Resource(id)) => Some(
                *tree
                    .by_id
                    .get(id)
                    .ok_or_else(|| err("advancement.advancementNotFound", &[id]))?,
            ),
            _ => None,
        };
        let player = Text::literal(PLAYER_NAME);
        let players = targets.len();
        if let (Some(i), Some(Arg::Str(criterion))) = (index, ctx.get("criterion")) {
            let a = &tree.list[i];
            let name = super::advancements::adv_name(a);
            if !a.criteria.iter().any(|c| &c.name == criterion) {
                return Err(CommandError::new(Text::translate(
                    "commands.advancement.criterionNotFound",
                    &[name, Text::literal(criterion.clone())],
                )));
            }
            let changed = if grant {
                self.grant_advancement(i, Some(criterion))
            } else {
                self.revoke_advancement(i, Some(criterion))
            };
            let to = if players == 1 { "one" } else { "many" };
            let who = if players == 1 {
                player
            } else {
                Text::literal(players.to_string())
            };
            let args = [Text::literal(criterion.clone()), name, who];
            if !changed {
                return Err(CommandError::new(Text::translate(
                    &format!("commands.advancement.{key}.criterion.to.{to}.failure"),
                    &args,
                )));
            }
            self.feedback(Text::translate(
                &format!("commands.advancement.{key}.criterion.to.{to}.success"),
                &args,
            ));
            return Ok(1);
        }
        // getAdvancements: parents first (nearest up), the target, then
        // the subtree depth first.
        let list: Vec<usize> = match index {
            None => (0..tree.list.len()).collect(),
            Some(i) => {
                let mut out = Vec::new();
                if ctx.has("until") || ctx.has("through") {
                    let mut p = tree.list[i].parent.as_ref();
                    while let Some(&pi) = p.and_then(|id| tree.by_id.get(id)) {
                        out.push(pi);
                        p = tree.list[pi].parent.as_ref();
                    }
                }
                out.push(i);
                if ctx.has("from") || ctx.has("through") {
                    let kids = tree.children();
                    fn add(kids: &[Vec<usize>], i: usize, out: &mut Vec<usize>) {
                        for &c in &kids[i] {
                            out.push(c);
                            add(kids, c, out);
                        }
                    }
                    add(&kids, i, &mut out);
                }
                out
            }
        };
        self.adv.quiet = index.is_none();
        let mut count = 0;
        for &i in &list {
            let changed = if grant {
                self.grant_advancement(i, None)
            } else {
                self.revoke_advancement(i, None)
            };
            count += i32::from(changed);
        }
        self.adv.quiet = false;
        let to = if players == 1 { "one" } else { "many" };
        let who = if players == 1 {
            player
        } else {
            Text::literal(players.to_string())
        };
        let (what, first) = if list.len() == 1 {
            ("one", super::advancements::adv_name(&tree.list[list[0]]))
        } else {
            ("many", Text::literal(list.len().to_string()))
        };
        let args = [first, who];
        if count == 0 {
            return Err(CommandError::new(Text::translate(
                &format!("commands.advancement.{key}.{what}.to.{to}.failure"),
                &args,
            )));
        }
        self.feedback(Text::translate(
            &format!("commands.advancement.{key}.{what}.to.{to}.success"),
            &args,
        ));
        Ok(count)
    }
    fn cmd_experience(&mut self, ctx: &Context) -> R<i32> {
        let levels = ctx.has("levels");
        let unit = if levels { "levels" } else { "points" };
        if ctx.has("query") {
            let _ = self.entities(ctx, "target", true)?;
            let e = &self.experience;
            let value = if levels {
                e.level
            } else {
                (e.progress * horizon_client::entities::Experience::needed(e.level) as f32) as i32
            };
            self.system_message(Text::translate(
                &format!("commands.experience.query.{unit}"),
                &[Text::literal(PLAYER_NAME), Text::literal(value.to_string())],
            ));
            return Ok(value);
        }
        let targets = self.entities(ctx, "target", true)?;
        let amount = Self::int(ctx, "amount").unwrap_or(0);
        let op = if ctx.has("add") { "add" } else { "set" };
        if op == "add" {
            if levels {
                self.experience.level = (self.experience.level + amount).max(0);
                if self.experience.level == 0 && amount < 0 {
                    self.experience.progress = 0.0;
                }
            } else {
                self.give_points(amount);
            }
        } else if levels {
            self.experience.level = amount;
        } else {
            let needed = horizon_client::entities::Experience::needed(self.experience.level);
            if amount > needed - 1 {
                return Err(err("commands.experience.set.points.invalid", &[]));
            }
            self.experience.progress = amount as f32 / needed as f32;
        }
        let key = format!(
            "commands.experience.{op}.{unit}.success.{}",
            if targets.len() == 1 {
                "single"
            } else {
                "multiple"
            }
        );
        self.feedback(Text::translate(
            &key,
            &[
                Text::literal(amount.to_string()),
                Text::literal(PLAYER_NAME),
            ],
        ));
        Ok(targets.len() as i32)
    }
    fn cmd_gamerule(&mut self, ctx: &Context) -> R<i32> {
        let Some(name) = ctx.literals.get(1).cloned() else {
            return Ok(0);
        };
        let Some(rule) = gamerules::rule(&name) else {
            return Ok(0);
        };
        let current = self.ui.rules.get(rule.name).unwrap_or(rule.default);
        let value = match ctx.get("value") {
            Some(Arg::Bool(b)) => Value::Bool(*b),
            Some(Arg::Int(i)) => Value::Int(*i),
            _ => {
                self.system_message(Text::tr("commands.gamerule.query", &[&rule.name, &current]));
                return Ok(match current {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                });
            }
        };
        if current == value {
            return Err(err("commands.gamerule.not_set", &[&rule.name, &value]));
        }
        self.ui.rules.set(rule.name, value);
        self.sync_rules();
        self.feedback(Text::tr("commands.gamerule.set", &[&rule.name, &value]));
        Ok(match value {
            Value::Bool(b) => i32::from(b),
            Value::Int(i) => i,
        })
    }
    fn cmd_difficulty(&mut self, ctx: &Context) -> R<i32> {
        let name = |d: u8| {
            Text::translate(
                &format!("options.difficulty.{}", DIFFICULTIES[d as usize]),
                &[],
            )
        };
        let Some(i) = DIFFICULTIES.iter().position(|d| ctx.has(d)) else {
            let current = self.ui.difficulty;
            self.system_message(Text::translate(
                "commands.difficulty.query",
                &[name(current)],
            ));
            return Ok(i32::from(current));
        };
        let i = i as u8;
        if self.ui.difficulty == i {
            return Err(CommandError::new(Text::translate(
                "commands.difficulty.failure",
                &[Text::literal(DIFFICULTIES[i as usize])],
            )));
        }
        self.ui.difficulty = i;
        self.sync_rules();
        if i == 0 {
            // Peaceful removes hostile mobs.
            self.mobs.retain(|m| !m.hostile());
        }
        self.feedback(Text::translate("commands.difficulty.success", &[name(i)]));
        Ok(0)
    }
    fn cmd_spawnpoint(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = if ctx.get("pos").is_some() {
            self.block_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        let yaw = ((rot[0] + 180.0).rem_euclid(360.0)) - 180.0;
        let pitch = rot[1].clamp(-90.0, 90.0);
        self.spawn = Some(Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ));
        let dim = Text::literal(self.dim.id());
        let p = pos.map(|v| Text::literal(v.to_string()));
        let mut args = vec![
            p[0].clone(),
            p[1].clone(),
            p[2].clone(),
            Text::literal(java_float(f64::from(yaw))),
            Text::literal(java_float(f64::from(pitch))),
            dim,
        ];
        let key = if targets.len() == 1 {
            args.push(Text::literal(PLAYER_NAME));
            "commands.spawnpoint.success.single"
        } else {
            args.push(Text::literal(targets.len().to_string()));
            "commands.spawnpoint.success.multiple"
        };
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }
    fn cmd_setworldspawn(&mut self, ctx: &Context) -> R<i32> {
        let pos = if ctx.get("pos").is_some() {
            self.block_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        if self.dim != Dim::Overworld {
            return Err(err("commands.setworldspawn.failure.not_overworld", &[]));
        }
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        self.level.spawn = Some([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
        let args = [
            Text::literal(pos[0].to_string()),
            Text::literal(pos[1].to_string()),
            Text::literal(pos[2].to_string()),
            Text::literal(java_float(f64::from(rot[0]))),
            Text::literal(java_float(f64::from(rot[1]))),
            Text::literal(self.dim.id()),
        ];
        self.feedback(Text::translate("commands.setworldspawn.success", &args));
        Ok(1)
    }
    fn item_matches(&self, predicate: &ItemInput, item: &str) -> bool {
        if predicate.any {
            return true;
        }
        if predicate.tag {
            let tag = predicate.id.trim_start_matches("minecraft:");
            return self
                .res
                .items
                .item_tags
                .get(tag)
                .or_else(|| self.res.items.item_tags.get(&predicate.id))
                .is_some_and(|s| {
                    s.contains(item) || s.contains(item.trim_start_matches("minecraft:"))
                });
        }
        predicate.id == item
    }
    fn cmd_clear(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let predicate = match ctx.get("item") {
            Some(Arg::Item(i)) => i.clone(),
            _ => ItemInput {
                id: "*".into(),
                tag: false,
                any: true,
                components: None,
            },
        };
        let max = Self::int(ctx, "maxCount").unwrap_or(-1);
        let counting = max == 0;
        // Inventory.clearOrCountMatchingItems: main, armor, crafting grid, carried.
        let mut removed = 0i32;
        let mut budget = if max < 0 { i32::MAX } else { max };
        let mut slots: Vec<&mut Option<ItemStack>> = Vec::new();
        slots.extend(self.inventory.iter_mut());
        slots.extend(self.armor.iter_mut());
        slots.extend(self.craft_grid.iter_mut());
        slots.push(&mut self.carried);
        for slot in slots {
            let Some(stack) = slot.as_mut() else { continue };
            if !(predicate.any || predicate.tag || predicate.id == *stack.item) && !predicate.tag {
                continue;
            }
            if predicate.tag {
                continue;
            }
            let take = if counting {
                stack.count as i32
            } else {
                (stack.count as i32).min(budget)
            };
            removed += take;
            if !counting {
                budget -= take;
                stack.count -= take as u32;
                if stack.count == 0 {
                    *slot = None;
                }
                if budget == 0 {
                    break;
                }
            }
        }
        if predicate.tag {
            // Tag predicates: match against item tags.
            let pred = predicate.clone();
            let mut budget = if max < 0 { i32::MAX } else { max };
            let items: Vec<(usize, String, u32)> = self
                .inventory
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.item.to_string(), s.count)))
                .collect();
            for (i, item, count) in items {
                if !self.item_matches(&pred, &item) {
                    continue;
                }
                let take = if counting {
                    count as i32
                } else {
                    (count as i32).min(budget)
                };
                removed += take;
                if !counting && let Some(s) = self.inventory[i].as_mut() {
                    budget -= take;
                    s.count -= take as u32;
                    if s.count == 0 {
                        self.inventory[i] = None;
                    }
                }
            }
        }
        if removed == 0 {
            return Err(if targets.len() == 1 {
                CommandError::new(Text::translate(
                    "clear.failed.single",
                    &[Text::literal(PLAYER_NAME)],
                ))
            } else {
                err("clear.failed.multiple", &[&targets.len()])
            });
        }
        let kind = if counting { "test" } else { "success" };
        self.feedback(Text::translate(
            &format!("commands.clear.{kind}.single"),
            &[
                Text::literal(removed.to_string()),
                Text::literal(PLAYER_NAME),
            ],
        ));
        Ok(removed)
    }
    fn cmd_locate(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let origin = self.source_position();
        if id == "locate.biome" {
            let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
                return Ok(0);
            };
            // Search loaded chunks around the player (BiomeSource.findClosestBiome3d
            // samples a 6400-block radius; Horizon searches what is generated).
            let (px, pz) = (origin[0].floor() as i32, origin[2].floor() as i32);
            let mut best: Option<([i32; 3], f64)> = None;
            for (&(cx, cz), chunk) in &self.world.chunks {
                for (dx, dz) in [(0, 0), (8, 8)] {
                    let (x, z) = (cx * 16 + dx, cz * 16 + dz);
                    let y = origin[1].floor() as i32;
                    let b = self.res.biomes.get(chunk.biome(x, y, z));
                    if b.name == biome {
                        let d = f64::from((x - px).pow(2) + (z - pz).pow(2));
                        if best.is_none_or(|(_, bd)| d < bd) {
                            best = Some(([x, y, z], d));
                        }
                    }
                }
            }
            let Some((p, d2)) = best else {
                return Err(CommandError::new(Text::tr(
                    "commands.locate.biome.not_found",
                    &[&biome],
                )));
            };
            let coords = Text::tr("chat.coordinates", &[&p[0], &p[1], &p[2]]);
            let link = Text::literal("[")
                .append(coords)
                .append("]")
                .with_style(&Style {
                    color: Some(text::GREEN),
                    suggest: Some(format!("/tp @s {} {} {}", p[0], p[1], p[2])),
                    ..Style::default()
                });
            let distance = d2.sqrt().floor() as i32;
            self.system_message(Text::translate(
                "commands.locate.biome.success",
                &[
                    Text::literal(biome),
                    link,
                    Text::literal(distance.to_string()),
                ],
            ));
            return Ok(distance);
        }
        let Some(Arg::Resource(s)) = ctx.get("structure").cloned() else {
            return Ok(0);
        };
        // LocateCommand.locateStructure: an id or a #tag of structures.
        let names = match s.strip_prefix('#') {
            Some(tag) => self.structure_tag(tag),
            None => vec![s.clone()],
        };
        let from = origin.map(|v| v.floor() as i32);
        let found = self.end.locator().and_then(|t| t.locate(&names, from, 100));
        let Some((name, p)) = found else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.structure.not_found",
                &[&s],
            )));
        };
        let coords = Text::translate(
            "chat.coordinates",
            &[
                Text::literal(p[0].to_string()),
                Text::literal("~"),
                Text::literal(p[2].to_string()),
            ],
        );
        let link = Text::literal("[")
            .append(coords)
            .append("]")
            .with_style(&Style {
                color: Some(text::GREEN),
                suggest: Some(format!("/tp @s {} ~ {}", p[0], p[2])),
                ..Style::default()
            });
        let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
        let distance = (dx * dx + dz * dz).sqrt().floor() as i32;
        let shown = if s.starts_with('#') {
            format!("{s} ({name})")
        } else {
            name
        };
        self.feedback(Text::translate(
            "commands.locate.structure.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    /// The structures of a `worldgen/structure` tag, nested tags expanded.
    pub(super) fn structure_tag(&self, tag: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = vec![tag.to_owned()];
        let mut seen = std::collections::HashSet::new();
        while let Some(t) = pending.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            let (ns, path) = t.split_once(':').unwrap_or(("minecraft", &t));
            let file = self
                .options
                .resources
                .join(format!("data/{ns}/tags/worldgen/structure/{path}.json"));
            let Ok(v) = std::fs::read(file)
                .map_err(|_| ())
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|_| ()))
            else {
                continue;
            };
            for e in v["values"].as_array().into_iter().flatten() {
                let id = e.as_str().or_else(|| e["id"].as_str()).unwrap_or_default();
                match id.strip_prefix('#') {
                    Some(nested) => pending.push(nested.to_owned()),
                    None if !id.is_empty() => out.push(id.to_owned()),
                    None => {}
                }
            }
        }
        out
    }
    fn cmd_say(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let raw = match ctx.get(if id == "me" { "action" } else { "message" }) {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let message = self.expand_selectors(&raw);
        // CommandSourceStack.getDisplayName: the executing entity's name.
        let name = match self.command_source.map(|s| s.entity) {
            Some(Some(t)) => self.target_name(t),
            Some(None) => Text::literal("Server"),
            None => Text::literal(PLAYER_NAME),
        };
        let line = match id {
            "say" => Text::translate("chat.type.announcement", &[name, message]),
            "me" => Text::translate("chat.type.emote", &[name, message]),
            _ => {
                let _ = self.entities(ctx, "targets", true)?;
                // Whispering to yourself: the incoming and outgoing lines.
                let out = Text::translate(
                    "commands.message.display.outgoing",
                    &[Text::literal(PLAYER_NAME), message.clone()],
                )
                .with_color(text::GRAY)
                .italic();
                self.system_message(out);
                Text::translate(
                    "commands.message.display.incoming",
                    &[Text::literal(PLAYER_NAME), message],
                )
                .with_color(text::GRAY)
                .italic()
            }
        };
        let focused = self.screen == Screen::Chat;
        self.ui
            .chat
            .add(&self.res.atlas, line, None, self.ticks, focused);
        Ok(1)
    }
    /// MessageArgument: `@p`-style selectors in a message become names.
    fn expand_selectors(&mut self, raw: &str) -> Text {
        let mut out = Text::empty();
        let mut rest = raw;
        while let Some(at) = rest.find('@') {
            out.push(&rest[..at]);
            let mut reader = Reader::new(&rest[at..]);
            match (horizon_client::command::ArgType::Entity {
                single: false,
                players: false,
            })
            .parse(&mut reader, &self.ui.registries)
            {
                Ok(Arg::Entity(s)) if reader.cursor > 1 => {
                    let found = self.select(&s);
                    for (i, t) in found.iter().enumerate() {
                        if i > 0 {
                            out.push(", ");
                        }
                        out.push(self.target_name(*t));
                    }
                    rest = &rest[at + reader.cursor..];
                }
                _ => {
                    out.push("@");
                    rest = &rest[at + 1..];
                }
            }
        }
        out.push(rest);
        out
    }
}
