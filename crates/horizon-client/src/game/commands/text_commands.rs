//! `/tellraw` (TellRawCommand) and `/title` (TitleCommand).
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::text::Text;

impl Game<'_> {
    /// ComponentArgument with selectors resolved for the source
    /// (ComponentUtils.updateForEntity: names joined by ", ").
    fn component_arg(&mut self, ctx: &Context, name: &str) -> R<Text> {
        let Some(Arg::Str(json)) = ctx.get(name) else {
            return Ok(Text::empty());
        };
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| err("argument.component.invalid", &[&e]))?;
        // NbtContents.resolve: the path's values in the entity, block or
        // storage data, rich-styled (or plain, or interpreted), ", "-joined.
        let nbt_values = std::cell::RefCell::new(Vec::new());
        {
            let mut collect = |o: &serde_json::Map<String, serde_json::Value>| {
                if o.contains_key("nbt") {
                    nbt_values.borrow_mut().push(o.clone());
                }
            };
            fn walk(
                v: &serde_json::Value,
                f: &mut dyn FnMut(&serde_json::Map<String, serde_json::Value>),
            ) {
                match v {
                    serde_json::Value::Object(o) => {
                        f(o);
                        for (_, x) in o {
                            walk(x, f);
                        }
                    }
                    serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, f)),
                    _ => {}
                }
            }
            walk(&value, &mut collect);
        }
        let mut resolved: Vec<(serde_json::Map<String, serde_json::Value>, Text)> = Vec::new();
        for o in nbt_values.into_inner() {
            let t = self.resolve_nbt_component(&o);
            resolved.push((o, t));
        }
        let mut resolve = |raw: &str| -> Text {
            let mut reader = horizon_client::command::Reader::new(raw);
            let entities = horizon_client::command::args::ArgType::Entity {
                single: false,
                players: false,
            };
            let Ok(Arg::Entity(sel)) = entities.parse(&mut reader, &self.ui.registries) else {
                return Text::literal(raw);
            };
            let found = self.select(&sel);
            let mut out = Text::empty();
            for (i, t) in found.into_iter().enumerate() {
                if i > 0 {
                    out.push(Text::literal(", "));
                }
                out.push(self.target_name(t));
            }
            out
        };
        Ok(horizon_client::component::to_text_with(
            &value,
            &mut resolve,
            &mut |o| {
                resolved
                    .iter()
                    .find(|(k, _)| k == o)
                    .map(|(_, t)| t.clone())
            },
        ))
    }
    /// The text of an `nbt` component.
    fn resolve_nbt_component(&mut self, o: &serde_json::Map<String, serde_json::Value>) -> Text {
        use horizon_world::nbt::Tag;
        let path = o.get("nbt").and_then(|v| v.as_str()).unwrap_or("");
        let mut sources: Vec<Tag> = Vec::new();
        if let Some(sel) = o.get("entity").and_then(|v| v.as_str()) {
            let mut reader = horizon_client::command::Reader::new(sel);
            let ty = horizon_client::command::args::ArgType::Entity {
                single: false,
                players: false,
            };
            if let Ok(Arg::Entity(s)) = ty.parse(&mut reader, &self.ui.registries) {
                for t in self.select(&s) {
                    if let Ok(tag) = self.data_of(&super::data::DataRef::Entity(t)) {
                        sources.push(tag);
                    }
                }
            }
        } else if let Some(id) = o.get("storage").and_then(|v| v.as_str()) {
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            sources.push(self.score.storage.get(&id));
        } else if let Some(pos) = o.get("block").and_then(|v| v.as_str()) {
            let p: Vec<i32> = pos
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if let [x, y, z] = p[..]
                && let Ok(tag) = self.data_of(&super::data::DataRef::Block([x, y, z]))
            {
                sources.push(tag);
            }
        }
        let Ok(compiled) = horizon_client::nbt_path::NbtPath::parse(path) else {
            return Text::empty();
        };
        let plain = o.get("plain").and_then(|v| v.as_bool()) == Some(true);
        let interpret = o.get("interpret").and_then(|v| v.as_bool()) == Some(true);
        let mut out = Text::empty();
        let mut first = true;
        for data in &sources {
            for tag in compiled.get(data).unwrap_or_default() {
                if !first {
                    out.push(Text::literal(", "));
                }
                first = false;
                let piece = if interpret {
                    horizon_client::scoreboard::comp_text(&horizon_client::scoreboard::tag_to_json(
                        &tag,
                    ))
                } else if plain {
                    Text::literal(horizon_client::snbt::pretty(&tag).plain())
                } else {
                    horizon_client::snbt::pretty(&tag)
                };
                out.push(piece);
            }
        }
        out
    }
    fn player_targets(&mut self, ctx: &Context) -> R<usize> {
        // Only the local player can receive messages and titles.
        let targets = self.entities(ctx, "targets", true)?;
        Ok(targets.iter().filter(|t| **t == Target::Player).count())
    }
    pub(super) fn cmd_tellraw(&mut self, ctx: &Context) -> R<i32> {
        let n = self.player_targets(ctx)?;
        let message = self.component_arg(ctx, "message")?;
        if n > 0 {
            self.system_message(message);
        }
        Ok(n as i32)
    }
    pub(super) fn cmd_title(&mut self, ctx: &Context) -> R<i32> {
        let n = self.player_targets(ctx)?;
        let name = self.target_name(Target::Player);
        let single = n == 1;
        let key = if ctx.has("clear") {
            self.titles.clear();
            "cleared"
        } else if ctx.has("reset") {
            self.titles.clear();
            self.titles.reset_times();
            "reset"
        } else if ctx.has("times") {
            let get = |k: &str| match ctx.get(k) {
                Some(Arg::Int(v)) => *v,
                _ => -1,
            };
            self.titles
                .set_times(get("fadeIn"), get("stay"), get("fadeOut"));
            "times"
        } else {
            let text = self.component_arg(ctx, "title")?;
            if ctx.has("subtitle") {
                self.titles.set_subtitle(text);
                "show.subtitle"
            } else if ctx.has("actionbar") {
                self.titles.set_overlay(text, false);
                "show.actionbar"
            } else {
                self.titles.set_title(text);
                "show.title"
            }
        };
        let message = if single {
            Text::translate(&format!("commands.title.{key}.single"), &[name])
        } else {
            Text::translate(
                &format!("commands.title.{key}.multiple"),
                &[Text::literal(n.to_string())],
            )
        };
        self.feedback(message);
        Ok(n as i32)
    }
}
