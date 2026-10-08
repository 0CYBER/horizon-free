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
        Ok(horizon_client::component::to_text(&value, &mut resolve))
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
                self.titles.subtitle = Some(text);
                "show.subtitle"
            } else if ctx.has("actionbar") {
                self.titles.set_overlay(text);
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
