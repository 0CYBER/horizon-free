//! A bug report sent from the game (the pause menu's Report Bugs), checked and written as a GitHub issue.
//!
//! The game fills in the technical part itself: the machine, the settings, what happened in the last seconds,
//! the end of the log and, for a performance report, the measurement and what most likely explains it. The
//! player writes a sentence or two and sees this whole text before sending it. Every report gets the label
//! `from-game`; a bug also gets `bug` and the area the player picked, a performance report `perf`.
use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::github::{Issue, neutralise};
use crate::intake::clean;

/// The kind its queue entries have (`hub_queue.kind`).
pub const KIND: &str = "game";
/// The largest body accepted: the log's end at its limit and the rest, as JSON.
pub const MAX_BODY: usize = 128 * 1024;
/// The label every report from the game gets.
pub const LABEL: &str = "from-game";
/// The areas a player may pick for a bug: the repository's area labels.
pub const AREAS: [&str; 10] = [
    "blocks",
    "mobs",
    "rendering",
    "audio",
    "ui",
    "multiplayer",
    "saves",
    "worldgen",
    "chunks",
    "commands",
];
/// A measurement's capture, opened on the website's performance viewer (`horizon-perf-share`): its path there.
const CAPTURE_PATH: &str = "/perf/c/";
/// GitHub refuses an issue body over 65,536 characters: the log's end is cut to keep under this.
const MAX_ISSUE_CHARS: usize = 60_000;

/// A text field: its name, whether it is needed, its length in characters, whether it may hold lines.
struct Text(&'static str, bool, RangeInclusive<usize>, bool);
/// A list field: its name, how many entries, and each entry's length.
struct List(&'static str, RangeInclusive<usize>, RangeInclusive<usize>);
/// A table field (`[[label, value], ...]`): its name, how many rows.
struct Table(&'static str, RangeInclusive<usize>);

const TEXTS: &[Text] = &[
    Text("title", true, 5..=120, false),
    Text("expected", false, 0..=500, false),
    Text("version", true, 3..=80, false),
    // The newer build the game knew of when it sent this: the report is kept, and says so.
    Text("outdated", false, 0..=80, false),
    Text("name", false, 0..=64, false),
    Text("log", false, 0..=40_000, true),
];
const LISTS: &[List] = &[
    List("context", 0..=30, 1..=200),
    List("likely", 0..=10, 1..=200),
];
const TABLES: &[Table] = &[Table("system", 1..=60), Table("settings", 0..=40)];

fn text(body: &Map<String, Value>, rule: &Text) -> Result<Option<String>, String> {
    let Text(name, needed, length, lines) = rule;
    let value = match body.get(*name) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => clean(s, *lines),
        Some(_) => return Err(format!("{name} is text")),
    };
    if value.is_empty() && !needed {
        return Ok(None);
    }
    if !length.contains(&value.chars().count()) {
        return Err(format!(
            "{name}: {} to {} characters",
            length.start(),
            length.end()
        ));
    }
    Ok(Some(value))
}

fn strings(value: &Value, name: &str) -> Result<Vec<String>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(clean(s, false)),
                _ => Err(format!("{name} holds text")),
            })
            .collect(),
        _ => Err(format!("{name} is a list")),
    }
}

/// The request's fields checked: the cleaned object, or what is wrong.
pub fn check(body: &Value) -> Result<Map<String, Value>, String> {
    let body = body.as_object().ok_or("send a JSON object")?;
    let mut out = Map::new();
    let category = body.get("category").and_then(Value::as_str).unwrap_or("");
    if !["bug", "performance"].contains(&category) {
        return Err("category: bug or performance".into());
    }
    out.insert("category".into(), category.into());
    let bug = category == "bug";
    // A bug needs the player's sentence; a performance report says what it measured instead.
    let own = [
        Text("happened", bug, if bug { 10..=500 } else { 0..=500 }, false),
        Text(
            "measurement",
            !bug,
            if bug { 0..=6000 } else { 10..=6000 },
            true,
        ),
    ];
    for rule in TEXTS.iter().chain(&own) {
        if let Some(value) = text(body, rule)? {
            out.insert(rule.0.into(), value.into());
        }
    }
    if let Some(area) = body
        .get("area")
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty())
    {
        if !bug || !AREAS.contains(&area) {
            return Err(format!("area: one of {}, for a bug", AREAS.join(", ")));
        }
        out.insert("area".into(), area.into());
    }
    for List(name, count, length) in LISTS {
        let items = strings(body.get(*name).unwrap_or(&Value::Null), name)?;
        if !count.contains(&items.len())
            || items.iter().any(|i| !length.contains(&i.chars().count()))
        {
            return Err(format!(
                "{name}: up to {} entries of {} characters",
                count.end(),
                length.end()
            ));
        }
        if !items.is_empty() {
            out.insert((*name).into(), items.into());
        }
    }
    for Table(name, count) in TABLES {
        let rows: &[Value] = match body.get(*name) {
            None | Some(Value::Null) => &[],
            Some(Value::Array(rows)) => rows,
            Some(_) => return Err(format!("{name} is a list of [label, value]")),
        };
        let mut kept = Vec::with_capacity(rows.len());
        for row in rows {
            let pair = strings(row, name)?;
            let [label, value] = pair.as_slice() else {
                return Err(format!("{name} is a list of [label, value]"));
            };
            if !(1..=40).contains(&label.chars().count()) || value.chars().count() > 200 {
                return Err(format!(
                    "{name}: labels up to 40 characters, values up to 200"
                ));
            }
            kept.push(Value::from(vec![label.clone(), value.clone()]));
        }
        if !count.contains(&kept.len()) {
            return Err(format!("{name}: {} to {} rows", count.start(), count.end()));
        }
        if !kept.is_empty() {
            out.insert((*name).into(), kept.into());
        }
    }
    if let Some(capture) = body
        .get("capture")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    {
        let prefix = format!("{}{CAPTURE_PATH}", crate::WEBSITE);
        let id = capture.strip_prefix(&prefix).unwrap_or("");
        if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!("capture: a link under {prefix}"));
        }
        out.insert("capture".into(), capture.into());
    }
    Ok(out)
}

/// Text in a fenced code block: GitHub links and renders nothing there, so only a backtick could end it.
fn code(text: &str) -> String {
    format!("```text\n{}\n```", text.replace('`', "'"))
}

/// A table cell: one line, no column breaks.
fn cell(text: &str) -> String {
    neutralise(text).replace('|', "/").replace('\n', " ")
}

fn table(rows: &[Value]) -> String {
    let mut out = String::from("| | |\n|---|---|\n");
    for row in rows {
        let label = cell(row[0].as_str().unwrap_or_default());
        let value = cell(row[1].as_str().unwrap_or_default());
        let value = if value.is_empty() {
            "not given by the system".into()
        } else {
            value
        };
        out.push_str(&format!("| {label} | {value} |\n"));
    }
    out
}

fn bullets(items: &[Value]) -> String {
    items
        .iter()
        .map(|i| format!("- {}", neutralise(i.as_str().unwrap_or_default())))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The issue for a report's fields (as [`check`] left them).
pub fn issue(fields: &Map<String, Value>) -> Issue {
    let get = |name: &str| fields.get(name).and_then(Value::as_str).unwrap_or_default();
    let list = |name: &str| {
        fields
            .get(name)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let bug = get("category") == "bug";
    let mut parts = Vec::new();
    let mut said = Vec::new();
    if !get("happened").is_empty() {
        said.push(format!(
            "**What happened:** {}",
            neutralise(get("happened"))
        ));
    }
    if !get("expected").is_empty() {
        said.push(format!(
            "**What I expected:** {}",
            neutralise(get("expected"))
        ));
    }
    if !said.is_empty() {
        parts.push(
            said.iter()
                .map(|l| format!("> {l}"))
                .collect::<Vec<_>>()
                .join("\n>\n"),
        );
    }
    let mut version = format!("**Version:** {}", neutralise(get("version")));
    if !get("outdated").is_empty() {
        version.push_str(&format!(
            "\n**Older build:** {} was already out when this was sent.",
            neutralise(get("outdated"))
        ));
    }
    parts.push(version);
    if !list("likely").is_empty() {
        parts.push(format!("### Most likely\n\n{}", bullets(&list("likely"))));
    }
    if !get("measurement").is_empty() {
        let mut measured = format!("### Measurement\n\n{}", code(get("measurement")));
        if !get("capture").is_empty() {
            measured.push_str(&format!(
                "\n\n[The whole capture]({}) (kept 30 days)",
                get("capture")
            ));
        }
        parts.push(measured);
    }
    parts.push(format!("### Machine\n\n{}", table(&list("system"))));
    if !list("settings").is_empty() {
        parts.push(format!("### Settings\n\n{}", table(&list("settings"))));
    }
    if !list("context").is_empty() {
        parts.push(format!(
            "### The last seconds\n\n{}",
            bullets(&list("context"))
        ));
    }
    let name = neutralise(get("name")).replace(['*', '_', '~', '|', '[', ']'], "");
    let by = if name.is_empty() {
        String::new()
    } else {
        format!(" by **{name}**")
    };
    let footer = format!(
        "Reported from the game{by}\n\n<sub>Opened by Horizon's Report Bugs screen. The player saw this text before \
         sending it; they have this issue's link and don't get GitHub notifications.</sub>"
    );
    let head = parts.join("\n\n");
    let log = get("log");
    let body = if log.is_empty() {
        format!("{head}\n\n{footer}")
    } else {
        // The log's end matters most: its start goes when the issue would be too long.
        let room =
            MAX_ISSUE_CHARS.saturating_sub(head.chars().count() + footer.chars().count() + 200);
        let skip = log.chars().count().saturating_sub(room);
        let kept: String = log.chars().skip(skip).collect();
        format!(
            "{head}\n\n<details><summary>The end of the log</summary>\n\n{}\n\n</details>\n\n{footer}",
            code(&kept)
        )
    };
    let mut labels = vec![LABEL.to_owned()];
    if bug {
        labels.push("bug".into());
        if !get("area").is_empty() {
            labels.push(get("area").into());
        }
    } else {
        labels.push("perf".into());
    }
    Issue {
        title: neutralise(get("title")),
        body,
        labels,
    }
}
