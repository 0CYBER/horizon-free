//! `/forceload`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::Context,
    scoreboard_cmd::{Holder, Tracker},
    text::Text,
};
use horizon_core::position::ChunkPos;

/// What `/forceload` needs from the game: the forced columns and the position arguments.
pub trait ForceloadHost: CommandHost {
    /// The id of the dimension in place, as the messages name it.
    fn dimension_id(&self) -> String;
    /// The columns forced by command, in no particular order.
    fn commanded_chunks(&self) -> Vec<(i32, i32)>;
    /// Whether the column holding chunk `key` is forced by command.
    fn is_commanded(&self, key: (i32, i32)) -> bool;
    /// `CommandSourceStack` failure feedback: the source is told, nothing is broadcast.
    fn command_failure(&mut self, message: Text);
    /// A column-position argument (`BlockPosArgument`'s x and z), floored.
    fn column_arg(&self, ctx: &Context, name: &str) -> R<[i32; 2]>;
    /// Drops every command-forced column.
    fn clear_forced(&mut self);
    /// Forces or releases chunk `(x, z)`, loading it when forced. True when it changed.
    fn set_forced_loaded(&mut self, x: i32, z: i32, forced: bool) -> bool;
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn chunk_text(x: i32, z: i32) -> Text {
    Text::literal(format!("[{x}, {z}]"))
}
fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}

/// `ForceLoadCommand.changeForceLoad`: update and load each newly forced column in order.
pub fn forceload(host: &mut dyn ForceloadHost, id: &str, ctx: &Context) -> R<i32> {
    let dim = Text::literal(host.dimension_id());
    match id {
        "forceload.query" if ctx.get("pos").is_none() => {
            let mut forced: Vec<(i32, i32)> = host.commanded_chunks();
            // ForceLoadCommand sorts its LongStream before formatting chunks.
            forced.sort_unstable_by_key(|&(x, z)| ChunkPos::new(x, z).pack());
            if forced.is_empty() {
                host.command_failure(Text::translate("commands.forceload.added.none", &[dim]));
                return Ok(0);
            }
            let list = forced
                .iter()
                .map(|(x, z)| format!("[{x}, {z}]"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = if forced.len() == 1 {
                Text::translate(
                    "commands.forceload.list.single",
                    &[dim, Text::literal(list)],
                )
            } else {
                Text::translate(
                    "commands.forceload.list.multiple",
                    &[num(forced.len()), dim, Text::literal(list)],
                )
            };
            host.feedback(message);
            Ok(forced.len() as i32)
        }
        "forceload.query" => {
            let [x, z] = host.column_arg(ctx, "pos")?;
            let key = (x >> 4, z >> 4);
            if host.is_commanded(key) {
                host.feedback(Text::translate(
                    "commands.forceload.query.success",
                    &[chunk_text(key.0, key.1), dim],
                ));
                Ok(1)
            } else {
                Err(host.error(
                    "commands.forceload.query.failure",
                    &[&chunk_text(key.0, key.1).plain(), &dim.plain()],
                ))
            }
        }
        "forceload.remove.all" => {
            host.clear_forced();
            host.feedback(Text::translate("commands.forceload.removed.all", &[dim]));
            Ok(0)
        }
        _ => {
            let add = id == "forceload.add";
            let from = host.column_arg(ctx, "from")?;
            let to = if ctx.get("to").is_some() {
                host.column_arg(ctx, "to")?
            } else {
                from
            };
            let (min_x, max_x) = (from[0].min(to[0]), from[0].max(to[0]));
            let (min_z, max_z) = (from[1].min(to[1]), from[1].max(to[1]));
            if min_x < -30_000_000
                || min_z < -30_000_000
                || max_x >= 30_000_000
                || max_z >= 30_000_000
            {
                return Err(host.error("argument.pos.outofworld", &[]));
            }
            let (cx0, cx1, cz0, cz1) = (min_x >> 4, max_x >> 4, min_z >> 4, max_z >> 4);
            let count = i64::from(cx1 - cx0 + 1) * i64::from(cz1 - cz0 + 1);
            if count > 256 {
                return Err(host.error("commands.forceload.toobig", &[&256, &count]));
            }
            let mut tracker = Tracker::default();
            for x in cx0..=cx1 {
                for z in cz0..=cz1 {
                    let changed = host.set_forced_loaded(x, z, add);
                    tracker.track(&holder(chunk_text(x, z)), i32::from(changed));
                }
            }
            let (lo, hi) = (chunk_text(cx0, cz0), chunk_text(cx1, cz1));
            let verb = if add { "added" } else { "removed" };
            let (message, total) = tracker.message(
                false,
                Some(host.error(&format!("commands.forceload.{verb}.failure"), &[])),
                |h, _| {
                    Text::translate(
                        &format!("commands.forceload.{verb}.single"),
                        &[h.text.clone(), dim.clone()],
                    )
                },
                |c, _| {
                    Text::translate(
                        &format!("commands.forceload.{verb}.multiple"),
                        &[num(c), dim.clone(), lo.clone(), hi.clone()],
                    )
                },
            )?;
            host.feedback(message);
            Ok(total)
        }
    }
}
