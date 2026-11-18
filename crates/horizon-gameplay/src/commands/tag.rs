//! `/tag`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    scoreboard_cmd::{Holder, Tracker, format_list},
    text::Text,
};

/// What `/tag` needs from the game. The host keeps the entities the command acts on and
/// names them by their place in that list, so a `Target` never crosses this trait.
pub trait TagHost: CommandHost {
    /// The command's `targets` (`EntityArgument.getEntities`), kept by the host. With
    /// `skip_knots` a natural leash knot is left out (and a selector that found only knots
    /// fails). Returns how many there are.
    fn select_targets(&mut self, ctx: &Context, skip_knots: bool) -> R<usize>;
    /// `Entity.getTags` of the `index`th target.
    fn tags_of(&self, index: usize) -> Vec<String>;
    /// The name of the `index`th target.
    fn target_name(&self, index: usize) -> Text;
    /// Writes the tags of the `index`th target.
    fn set_tags(&mut self, index: usize, tags: &[String]);
}

/// `/tag <targets> add|remove|list`.
pub fn tag(host: &mut dyn TagHost, id: &str, ctx: &Context) -> R<i32> {
    // A natural knot is listed but keeps no tags yet: `tag add`/`remove` leave it out.
    let count = host.select_targets(ctx, matches!(id, "tag.add" | "tag.remove"))?;
    let name = match ctx.get("name") {
        Some(Arg::Str(n)) => n.clone(),
        _ => String::new(),
    };
    let mut tracker = Tracker::default();
    let mut all: Vec<String> = Vec::new();
    for index in 0..count {
        let mut tags = host.tags_of(index);
        let h = Holder {
            name: String::new(),
            text: host.target_name(index),
            display: None,
        };
        match id {
            "tag.add" => {
                let added = tags.len() < 1024 && !tags.contains(&name);
                if added {
                    tags.push(name.clone());
                    host.set_tags(index, &tags);
                }
                tracker.track(&h, i32::from(added));
            }
            "tag.remove" => {
                let before = tags.len();
                tags.retain(|x| x != &name);
                let removed = tags.len() != before;
                if removed {
                    host.set_tags(index, &tags);
                }
                tracker.track(&h, i32::from(removed));
            }
            _ => {
                tracker.track(&h, tags.len() as i32);
                for tag in tags {
                    if !all.contains(&tag) {
                        all.push(tag);
                    }
                }
            }
        }
    }
    let num = |n: usize| Text::literal(n.to_string());
    let n = name.clone();
    let (message, total) = match id {
        "tag.add" => tracker.message(
            false,
            Some(host.error("commands.tag.add.failed", &[])),
            |h, _| {
                Text::translate(
                    "commands.tag.add.success.single",
                    &[Text::literal(n.clone()), h.text.clone()],
                )
            },
            |c, _| {
                Text::translate(
                    "commands.tag.add.success.multiple",
                    &[Text::literal(n.clone()), num(c)],
                )
            },
        ),
        "tag.remove" => tracker.message(
            false,
            Some(host.error("commands.tag.remove.failed", &[])),
            |h, _| {
                Text::translate(
                    "commands.tag.remove.success.single",
                    &[Text::literal(n.clone()), h.text.clone()],
                )
            },
            |c, _| {
                Text::translate(
                    "commands.tag.remove.success.multiple",
                    &[Text::literal(n.clone()), num(c)],
                )
            },
        ),
        _ => {
            let count = all.len();
            let message = if all.is_empty() {
                tracker
                    .message(
                        false,
                        None,
                        |h, _| {
                            Text::translate(
                                "commands.tag.list.single.empty",
                                std::slice::from_ref(&h.text),
                            )
                        },
                        |c, _| Text::translate("commands.tag.list.multiple.empty", &[num(c)]),
                    )?
                    .0
            } else {
                let list = {
                    let mut sorted = all.clone();
                    sorted.sort();
                    format_list(
                        sorted
                            .into_iter()
                            .map(|s| Text::literal(s).with_color(horizon_client::text::GREEN))
                            .collect(),
                    )
                };
                // `TagCommand.listTags` tracks each entity's tag count and
                // sends with `sendFeedback`'s default element type, NON_ZERO:
                // the single form names the one entity that has tags.
                tracker
                    .message(
                        false,
                        None,
                        |h, _| {
                            Text::translate(
                                "commands.tag.list.single.success",
                                &[h.text.clone(), num(count), list.clone()],
                            )
                        },
                        |c, _| {
                            Text::translate(
                                "commands.tag.list.multiple.success",
                                &[num(c), num(count), list.clone()],
                            )
                        },
                    )?
                    .0
            };
            // `TagCommand.listTags` reports with `sendFeedback(source, false, …)`:
            // the operators are not told, unlike `addTag`/`removeTag` (`true`).
            host.feedback_unbroadcast(message);
            return Ok(count as i32);
        }
    }?;
    host.feedback(message);
    Ok(total)
}
