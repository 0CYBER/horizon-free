//! Headless transport for the pack selection parity gate: builds each
//! case's `horizon_client::packs::SelectionModel`, applies its actions and
//! writes both lists with each entry's select/unselect/move flags after
//! every action, and the committed selection.
#![forbid(unsafe_code)]
use horizon_client::packs::{Position, Selectable, SelectionModel};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn lists(m: &SelectionModel) -> Value {
    let side = |selected: bool| -> Value {
        let list = if selected { &m.selected } else { &m.unselected };
        (0..list.len())
            .map(|i| {
                let mut flags = String::new();
                for (on, c) in [
                    (m.can_select(selected, i), 's'),
                    (m.can_unselect(selected, i), 'u'),
                    (m.can_move_up(selected, i), '^'),
                    (m.can_move_down(selected, i), 'v'),
                ] {
                    if on {
                        flags.push(c);
                    }
                }
                json!([list[i].id, flags])
            })
            .collect()
    };
    json!({"unselected": side(false), "selected": side(true)})
}

pub fn main() -> Result<()> {
    let args: Vec<PathBuf> = crate::args_os().skip(1).map(PathBuf::from).collect();
    let [inputs, output] = args.as_slice() else {
        return Err("usage: pack_selection_fixture INPUTS OUTPUT".into());
    };
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let c: Value = serde_json::from_str(line)?;
        let packs = c["packs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Selectable {
                id: p["id"].as_str().unwrap_or("").to_owned(),
                required: p["required"].as_bool().unwrap_or(false),
                position: if p["position"] == "top" {
                    Position::Top
                } else {
                    Position::Bottom
                },
                fixed: p["fixed"].as_bool().unwrap_or(false),
                compatible: p["compat"] == "compatible",
            })
            .collect();
        let selected: Vec<String> = c["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        let mut m = SelectionModel::new(packs, &selected);
        let mut steps = vec![lists(&m)];
        for op in c["ops"].as_array().into_iter().flatten() {
            let sel = op["list"] == "selected";
            let len = if sel {
                m.selected.len()
            } else {
                m.unselected.len()
            };
            if len > 0 {
                let i = op["index"].as_i64().unwrap_or(0).rem_euclid(len as i64) as usize;
                match op["op"].as_str() {
                    Some("activate") => {
                        m.activate(sel, i, op["confirm"].as_bool().unwrap_or(false));
                    }
                    Some("up") => {
                        m.move_by(sel, i, true);
                    }
                    Some("down") => {
                        m.move_by(sel, i, false);
                    }
                    _ => return Err(format!("unknown op {op}").into()),
                }
            }
            steps.push(lists(&m));
        }
        writeln!(out, "{}", json!({"steps": steps, "committed": m.commit()}))?;
    }
    Ok(())
}
