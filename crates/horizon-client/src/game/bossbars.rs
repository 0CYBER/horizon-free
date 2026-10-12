//! `/bossbar` (BossBarCommands) and drawing the custom boss bars.
use super::{
    Game,
    commands::{R, err},
};
use horizon_client::{
    bossbar::{self, BossBars},
    command::{Arg, Context},
    hud_overlays, save,
    scoreboard::{self as sb, Comp, comp_text},
    scoreboard_cmd::format_list,
    text::Text,
};
use horizon_world::nbt::Tag;

#[derive(Default)]
pub(super) struct BossState {
    pub bars: BossBars,
    loaded: bool,
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

impl Game<'_> {
    fn bossbar_path(&self) -> std::path::PathBuf {
        self.world_root
            .join("data/minecraft/custom_boss_events.dat")
    }
    pub(super) fn load_bossbars(&mut self) {
        if self.bosses.loaded || self.remote.is_some() {
            return;
        }
        self.bosses.loaded = true;
        if let Some(root) = save::read_dat(&self.bossbar_path())
            && let Some(data) = root.get("data")
        {
            self.bosses.bars = BossBars::unpack(data, self.level.player_uuid);
        }
        self.ui.registries.bossbars = self.bosses.bars.ids();
    }
    pub(super) fn save_bossbars(&self) {
        if !self.bosses.loaded || self.remote.is_some() {
            return;
        }
        if self.bosses.bars.bars.is_empty() && !self.bossbar_path().exists() {
            return;
        }
        let _ = save::write_dat(
            &self.bossbar_path(),
            Tag::compound([
                ("data", self.bosses.bars.pack(self.level.player_uuid)),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]),
        );
    }
    pub(in crate::game) fn bossbar_exists(&self, id: &str) -> bool {
        self.bosses.bars.get(id).is_some()
    }
    /// `execute store ... bossbar <id> value|max`.
    pub(in crate::game) fn set_bossbar_value(&mut self, id: &str, value: bool, v: i32) {
        if let Some(b) = self.bosses.bars.get_mut(id) {
            if value {
                b.value = v.clamp(0, b.max);
            } else {
                b.max = v.max(1);
                b.value = b.value.min(b.max);
            }
        }
    }
    fn bar_id(ctx: &Context) -> String {
        match ctx.get("id") {
            Some(Arg::Resource(id)) => id.clone(),
            _ => String::new(),
        }
    }
    fn bar_checked(&self, ctx: &Context) -> R<String> {
        let id = Self::bar_id(ctx);
        if self.bosses.bars.get(&id).is_none() {
            return Err(err("commands.bossbar.unknown", &[&id]));
        }
        Ok(id)
    }
    pub(super) fn cmd_bossbar(&mut self, id: &str, ctx: &Context) -> R<i32> {
        self.load_bossbars();
        let result = self.bossbar_command(id, ctx);
        self.ui.registries.bossbars = self.bosses.bars.ids();
        result
    }
    fn bossbar_command(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "bossbar.add" => {
                let bar = Self::bar_id(ctx);
                if self.bosses.bars.get(&bar).is_some() {
                    return Err(err("commands.bossbar.create.failed", &[&bar]));
                }
                let name = self.component_json(ctx, "name")?;
                self.bosses.bars.add(&bar, name);
                let shown = self
                    .bosses
                    .bars
                    .get(&bar)
                    .map(|b| b.display_name())
                    .unwrap_or_default();
                self.feedback(Text::translate("commands.bossbar.create.success", &[shown]));
                return Ok(self.bosses.bars.bars.len() as i32);
            }
            "bossbar.list" => {
                let names: Vec<Text> = self
                    .bosses
                    .bars
                    .bars
                    .iter()
                    .map(horizon_client::bossbar::BossBar::display_name)
                    .collect();
                let n = names.len();
                if n == 0 {
                    self.feedback(Text::translate("commands.bossbar.list.bars.none", &[]));
                } else {
                    self.feedback(Text::translate(
                        "commands.bossbar.list.bars.some",
                        &[num(n), format_list(names)],
                    ));
                }
                return Ok(n as i32);
            }
            _ => {}
        }
        let bar = self.bar_checked(ctx)?;
        let shown = |g: &Self| {
            g.bosses
                .bars
                .get(&bar)
                .map(|b| b.display_name())
                .unwrap_or_default()
        };
        let last = ctx.literals.last().cloned().unwrap_or_default();
        match id {
            "bossbar.remove" => {
                let s = shown(self);
                self.bosses.bars.remove(&bar);
                self.feedback(Text::translate("commands.bossbar.remove.success", &[s]));
                Ok(self.bosses.bars.bars.len() as i32)
            }
            "bossbar.get" => {
                let b = self.bosses.bars.get(&bar).cloned().unwrap_or_else(|| {
                    horizon_client::bossbar::BossBar::new(&bar, sb::literal(&bar))
                });
                let s = b.display_name();
                Ok(match last.as_str() {
                    "value" => {
                        self.feedback(Text::translate(
                            "commands.bossbar.get.value",
                            &[s, num(b.value)],
                        ));
                        b.value
                    }
                    "max" => {
                        self.feedback(Text::translate(
                            "commands.bossbar.get.max",
                            &[s, num(b.max)],
                        ));
                        b.max
                    }
                    "visible" => {
                        let key = if b.visible {
                            "commands.bossbar.get.visible.visible"
                        } else {
                            "commands.bossbar.get.visible.hidden"
                        };
                        self.feedback(Text::translate(key, &[s]));
                        i32::from(b.visible)
                    }
                    _ => {
                        if b.player {
                            let name = self.target_name(super::commands::Target::Player);
                            self.feedback(Text::translate(
                                "commands.bossbar.get.players.some",
                                &[s, num(1), format_list(vec![name])],
                            ));
                        } else {
                            self.feedback(Text::translate(
                                "commands.bossbar.get.players.none",
                                &[s],
                            ));
                        }
                        i32::from(b.player)
                    }
                })
            }
            _ => self.bossbar_set(id, ctx, &bar, &last),
        }
    }
    fn bossbar_set(&mut self, id: &str, ctx: &Context, bar: &str, last: &str) -> R<i32> {
        let shown = |g: &Self| {
            g.bosses
                .bars
                .get(bar)
                .map(|b| b.display_name())
                .unwrap_or_default()
        };
        let int = |name: &str| match ctx.get(name) {
            Some(Arg::Int(v)) => *v,
            _ => 0,
        };
        match id {
            "bossbar.set.name" => {
                let name = self.component_json(ctx, "name")?;
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                if b.name == name {
                    return Err(err("commands.bossbar.set.name.unchanged", &[]));
                }
                b.name = name;
                let s = shown(self);
                self.feedback(Text::translate("commands.bossbar.set.name.success", &[s]));
                Ok(0)
            }
            "bossbar.set.color" | "bossbar.set.style" => {
                let color = id.ends_with("color");
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                let slot = if color { &mut b.color } else { &mut b.style };
                if slot == last {
                    return Err(err(
                        if color {
                            "commands.bossbar.set.color.unchanged"
                        } else {
                            "commands.bossbar.set.style.unchanged"
                        },
                        &[],
                    ));
                }
                *slot = last.to_owned();
                let s = shown(self);
                self.feedback(Text::translate(
                    if color {
                        "commands.bossbar.set.color.success"
                    } else {
                        "commands.bossbar.set.style.success"
                    },
                    &[s],
                ));
                Ok(0)
            }
            "bossbar.set.value" | "bossbar.set.max" => {
                let is_value = id.ends_with("value");
                let v = int(if is_value { "value" } else { "max" });
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                let slot = if is_value { &mut b.value } else { &mut b.max };
                if *slot == v {
                    return Err(err(
                        if is_value {
                            "commands.bossbar.set.value.unchanged"
                        } else {
                            "commands.bossbar.set.max.unchanged"
                        },
                        &[],
                    ));
                }
                *slot = v;
                let s = shown(self);
                self.feedback(Text::translate(
                    if is_value {
                        "commands.bossbar.set.value.success"
                    } else {
                        "commands.bossbar.set.max.success"
                    },
                    &[s, num(v)],
                ));
                Ok(v)
            }
            "bossbar.set.visible" => {
                let visible = matches!(ctx.get("visible"), Some(Arg::Bool(true)));
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                if b.visible == visible {
                    return Err(err(
                        if visible {
                            "commands.bossbar.set.visibility.unchanged.visible"
                        } else {
                            "commands.bossbar.set.visibility.unchanged.hidden"
                        },
                        &[],
                    ));
                }
                b.visible = visible;
                let s = shown(self);
                self.feedback(Text::translate(
                    if visible {
                        "commands.bossbar.set.visible.success.visible"
                    } else {
                        "commands.bossbar.set.visible.success.hidden"
                    },
                    &[s],
                ));
                Ok(0)
            }
            _ => {
                // players [<targets>]
                let include = if ctx.get("targets").is_some() {
                    !self.entities(ctx, "targets", true)?.is_empty()
                } else {
                    false
                };
                let b = self.bosses.bars.get_mut(bar).expect("checked");
                if b.player == include {
                    return Err(err("commands.bossbar.set.players.unchanged", &[]));
                }
                b.player = include;
                let s = shown(self);
                if include {
                    let name = self.target_name(super::commands::Target::Player);
                    self.feedback(Text::translate(
                        "commands.bossbar.set.players.success.some",
                        &[s, num(1), format_list(vec![name])],
                    ));
                } else {
                    self.feedback(Text::translate(
                        "commands.bossbar.set.players.success.none",
                        &[s],
                    ));
                }
                Ok(i32::from(include))
            }
        }
    }
    /// A component argument as JSON with its selectors resolved.
    fn component_json(&mut self, ctx: &Context, name: &str) -> R<Comp> {
        let Some(Arg::Str(json)) = ctx.get(name) else {
            return Ok(sb::literal(""));
        };
        let value: Comp =
            serde_json::from_str(json).map_err(|e| err("argument.component.invalid", &[&e]))?;
        Ok(self.resolve_component(value))
    }

    // ---- drawing ------------------------------------------------------------------

    /// The visible custom bars (`/bossbar`), after the built-in ones.
    pub(super) fn custom_bossbars(&self, bars: &mut Vec<hud_overlays::BossBar>) {
        for b in self
            .bosses
            .bars
            .bars
            .iter()
            .filter(|b| b.visible && b.player)
        {
            bars.push(hud_overlays::BossBar {
                name: comp_text(&b.name),
                progress: b.progress(),
                color: bossbar::COLORS
                    .iter()
                    .position(|c| *c == b.color)
                    .unwrap_or(0),
                overlay: bossbar::STYLES
                    .iter()
                    .position(|s| *s == b.style)
                    .unwrap_or(0),
            });
        }
    }
}
