//! `/data merge`, `get`, `remove` and `modify`, over one data accessor (entity, block or storage).
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    data_cmd::{self, Op, path_error},
    nbt_path::NbtPath,
    text::Text,
};
use horizon_core::nbt::Tag;

/// What `/data` needs from the game, for the accessor the command acts on (the host knows it,
/// so a `Target` never crosses this trait). Each method calls the game code it names.
pub trait DataHost: CommandHost {
    /// `DataAccessor.getData` of the target.
    fn data_of(&mut self) -> R<Tag>;
    /// `DataAccessor.setData` of the target.
    fn set_data_of(&mut self, tag: Tag) -> R<()>;
    /// The "modified" message of the target.
    fn modified_text(&self) -> Text;
    /// The query message of the target for `data` read at `path` (`None` for the whole data).
    fn query_text(&self, data: &Tag, path: Option<&NbtPath>) -> Text;
    /// The `get ... <scale>` message of the target.
    fn get_text(&self, path: &NbtPath, scale: f64, value: i32) -> Text;
    /// A path argument of the command.
    fn path_arg(&self, ctx: &Context, name: &str) -> R<NbtPath>;
    /// A message sent without the feedback gamerule check (`sendSuccess(.., false)`).
    fn system_feedback(&mut self, message: Text);
    /// The data of the command's source accessor (`modify ... from <source>`).
    fn source_data(&mut self, ctx: &Context) -> R<Tag>;
    /// `LootContextSources`' compute source: the provider's value in the command's loot
    /// context, as an int or a float tag.
    fn compute_source(&mut self, ctx: &Context, integer: bool) -> R<Tag>;
    /// `DataCommands.manipulateData` on the target (the game's own `data modify` entry).
    fn modify_data(&mut self, ctx: &Context) -> R<i32>;
    /// A storage target's data is the stored compound itself: an edit that changed it keeps
    /// even when the command then fails. Other targets' data is a copy, so nothing happens.
    fn keep_unsaved_storage(&mut self, data: Tag);
}

/// `DataCommands.mergeData`: literal values keep the modern argument
/// parser's decoded Tag contract through execution.
pub fn data_in(host: &mut dyn DataHost, id: &str, ctx: &Context) -> R<i32> {
    match id {
        "data.merge" => {
            let Some(Arg::Str(nbt)) = ctx.get("nbt") else {
                return Ok(0);
            };
            let nbt = horizon_client::command::snbt::parse_fully(nbt)?;
            let old = host.data_of()?;
            if horizon_client::nbt_path::too_deep(&nbt, 0) {
                return Err(host.error("arguments.nbtpath.too_deep", &[]));
            }
            let result = data_cmd::merged(&old, &nbt);
            if old == result {
                return Err(host.error("commands.data.merge.failed", &[]));
            }
            host.set_data_of(result)?;
            let message = host.modified_text();
            host.feedback(message);
            Ok(1)
        }
        "data.get" => {
            let data = host.data_of()?;
            if ctx.get("path").is_none() {
                let message = host.query_text(&data, None);
                host.system_feedback(message);
                return Ok(1);
            }
            let path = host.path_arg(ctx, "path")?;
            let tag = data_cmd::single_tag(&path, &data)?;
            if let Some(Arg::Double(scale)) = ctx.get("scale") {
                let value = data_cmd::scaled_value(&tag, &path, *scale)?;
                let message = host.get_text(&path, *scale, value);
                host.system_feedback(message);
                return Ok(value);
            }
            let value = data_cmd::get_value(&tag, &path)?;
            let message = host.query_text(&tag, Some(&path));
            host.system_feedback(message);
            Ok(value)
        }
        "data.remove" => {
            let path = host.path_arg(ctx, "path")?;
            let mut data = host.data_of()?;
            let count = path.remove(&mut data);
            if count == 0 {
                return Err(host.error("commands.data.merge.failed", &[]));
            }
            host.set_data_of(data)?;
            let message = host.modified_text();
            host.feedback(message);
            Ok(count as i32)
        }
        "data.modify" => host.modify_data(ctx),
        _ => Err(host.error("command.failed", &[])),
    }
}

/// `DataCommands.manipulateData`: literal sources use the same SNBT
/// decode contract as command admission; stored sources are unchanged.
pub fn data_modify(host: &mut dyn DataHost, ctx: &Context) -> R<i32> {
    let target_path = host.path_arg(ctx, "targetPath")?;
    let after_modify: Vec<&String> = ctx
        .literals
        .iter()
        .skip_while(|l| l.as_str() != "modify")
        .collect();
    let op = after_modify
        .iter()
        .find_map(|l| match l.as_str() {
            "prepend" => Some(Op::Prepend),
            "append" => Some(Op::Append),
            "set" => Some(Op::Set),
            "merge" => Some(Op::Merge),
            "insert" => Some(Op::Insert(match ctx.get("index") {
                Some(Arg::Int(i)) => *i,
                _ => 0,
            })),
            _ => None,
        })
        .unwrap_or(Op::Set);
    let sources: Vec<Tag> = if ctx.has("value") {
        let Some(Arg::Str(text)) = ctx.get("value") else {
            return Ok(0);
        };
        vec![horizon_client::command::snbt::parse_fully(text)?]
    } else if ctx.get("provider").is_some() {
        let integer = ctx.literals.iter().any(|l| l == "integer");
        vec![host.compute_source(ctx, integer)?]
    } else {
        let data = host.source_data(ctx)?;
        let tags = if ctx.get("sourcePath").is_some() {
            host.path_arg(ctx, "sourcePath")?
                .get(&data)
                .map_err(path_error)?
        } else {
            vec![data]
        };
        if ctx.has("string") {
            let start = match ctx.get("start") {
                Some(Arg::Int(i)) => Some(*i),
                _ => None,
            };
            let end = match ctx.get("end") {
                Some(Arg::Int(i)) => Some(*i),
                _ => None,
            };
            let mut out = Vec::new();
            for t in &tags {
                let text = data_cmd::as_text(t)?;
                let text = match start {
                    Some(s) => data_cmd::substring(&text, s, end)?,
                    None => text,
                };
                out.push(Tag::String(text.as_str().into()));
            }
            out
        } else {
            tags
        }
    };
    let mut data = host.data_of()?;
    let changed = data_cmd::modify(op, &mut data, &target_path, &sources)?;
    if changed == 0 {
        // StorageDataAccessor.getData is the stored compound itself: an
        // edit that changed it and still counted nothing (a wildcard set
        // that clears an array, then rejects the value) stays. An entity's
        // or a block's data is a copy.
        host.keep_unsaved_storage(data);
        return Err(host.error("commands.data.merge.failed", &[]));
    }
    host.set_data_of(data)?;
    let message = host.modified_text();
    host.feedback(message);
    Ok(changed as i32)
}
