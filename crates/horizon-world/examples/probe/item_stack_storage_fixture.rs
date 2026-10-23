use horizon_world::{
    component_patch::{Patch, Prototype},
    item_stack_storage::{CopyResult, ItemStackData},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = ItemStackData<u8, usize, i32>;
fn optional(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
}
fn patch(v: &Value) -> Patch<usize, i32> {
    Patch::new(v.as_array().unwrap().iter().map(|v| {
        (
            v[0].as_u64().unwrap() as usize,
            v[1].as_i64().map(|v| v as i32),
        )
    }))
}
fn patch_text(p: &Patch<usize, i32>) -> String {
    p.entries()
        .iter()
        .map(|(k, v)| format!("{k}={}", optional(*v)))
        .collect::<Vec<_>>()
        .join(",")
}
fn snapshot(s: &Stack, canonical: bool) -> String {
    let visible = (0..4)
        .map(|k| optional(s.component(&k).copied()))
        .collect::<Vec<_>>()
        .join(",");
    let entries = s
        .components
        .entries()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{} {} {} {} {} {} {} | {} | {} | {} | {}",
        canonical,
        if canonical {
            -1
        } else {
            i32::from(s.stored_item())
        },
        s.item(),
        s.count,
        s.count(),
        s.pop_time,
        s.is_empty(),
        visible,
        entries,
        patch_text(&s.components.as_patch()),
        patch_text(&s.visible_patch())
    )
}
fn all(out: &mut dyn Write, arena: &[Stack], slots: &[usize; 3]) -> io::Result<()> {
    for (i, id) in slots.iter().enumerate() {
        writeln!(out, "slot {i} {}", snapshot(&arena[*id], *id == 0))?;
    }
    for a in 0..3 {
        for b in a + 1..3 {
            let sa = &arena[slots[a]];
            let sb = &arena[slots[b]];
            writeln!(
                out,
                "equal {a} {b} {} {} {} {}",
                slots[a] == slots[b],
                sa.same_item(sb),
                sa.same_item_components(sb),
                sa.matches(sb)
            )?;
        }
    }
    Ok(())
}
fn append(arena: &mut Vec<Stack>, copy: CopyResult<Stack>) -> usize {
    match copy {
        CopyResult::Empty => 0,
        CopyResult::Stack(s) => {
            arena.push(s);
            arena.len() - 1
        }
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let prototypes: Vec<_> = f["prototypes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                Prototype::new(p.as_array().unwrap().iter().map(|v| {
                    (
                        v[0].as_u64().unwrap() as usize,
                        v[1].as_i64().unwrap() as i32,
                    )
                }))
            })
            .collect();
        let mut arena = vec![Stack::new(0, 0, 0, Prototype::new([]))];
        let mut slots = [0; 3];
        for (i, v) in f["stacks"].as_array().unwrap().iter().enumerate() {
            let item = v["item"].as_i64().unwrap();
            if item >= 0 {
                let mut s = Stack::new(
                    item as u8,
                    0,
                    v["count"].as_i64().unwrap() as i32,
                    prototypes[item as usize].clone(),
                );
                s.pop_time = v["pop"].as_i64().unwrap() as i32;
                s.components.restore_patch(&patch(&v["patch"]));
                arena.push(s);
                slots[i] = arena.len() - 1;
            }
        }
        all(&mut out, &arena, &slots)?;
        for op in f["ops"].as_array().unwrap() {
            let a = op["a"].as_u64().unwrap() as usize;
            let b = op["b"].as_u64().unwrap() as usize;
            let amount = op["amount"].as_i64().unwrap() as i32;
            let id = slots[a];
            let mut calls = 0;
            let result = (|| -> io::Result<()> {
                match op["kind"].as_str().unwrap() {
                    "count" => arena[id].count = amount,
                    "pop" => arena[id].pop_time = amount,
                    "grow" => arena[id].grow(amount),
                    "shrink" => arena[id].shrink(amount),
                    "limit" => arena[id].limit_size(amount),
                    "set" => {
                        let value = arena[id]
                            .components
                            .set(
                                op["key"].as_u64().unwrap() as usize,
                                op["value"].as_i64().map(|v| v as i32),
                            )
                            .map_err(io::Error::other)?;
                        writeln!(out, "old {}", optional(value))?;
                    }
                    "copy" | "copy-count" | "copy-clear" | "split" | "transmute" => {
                        let copy = match op["kind"].as_str().unwrap() {
                            "copy" => arena[id].copy(),
                            "copy-count" => arena[id].copy_with_count(amount),
                            "copy-clear" => arena[id].copy_and_clear(),
                            "split" => arena[id].split(amount),
                            _ => {
                                let item = op["item"].as_u64().unwrap() as usize;
                                arena[id].transmute_copy(
                                    item as u8,
                                    prototypes[item].clone(),
                                    amount,
                                )
                            }
                        };
                        slots[b] = append(&mut arena, copy);
                    }
                    "consume" | "consume-return" => {
                        let infinite = |s: &mut Stack| -> io::Result<bool> {
                            writeln!(out, "infinite | {}", snapshot(s, id == 0))?;
                            calls += 1;
                            if op["fail"].as_bool().unwrap() {
                                return Err(io::Error::other("injected callback failure"));
                            }
                            if op["mutate"].as_bool().unwrap() {
                                s.count = op["after_count"].as_i64().unwrap() as i32;
                                s.pop_time = op["after_pop"].as_i64().unwrap() as i32;
                                for pair in op["after_components"].as_array().unwrap() {
                                    s.components
                                        .set(
                                            pair[0].as_u64().unwrap() as usize,
                                            pair[1].as_i64().map(|v| v as i32),
                                        )
                                        .map_err(io::Error::other)?;
                                }
                            }
                            Ok(op["infinite"].as_bool().unwrap())
                        };
                        let actor = op["actor"].as_bool().unwrap();
                        if op["kind"] == "consume" {
                            arena[id].consume(amount, actor, infinite)?;
                        } else {
                            let copy = arena[id].consume_and_return(amount, actor, infinite)?;
                            slots[b] = append(&mut arena, copy);
                        }
                    }
                    _ => panic!("operation"),
                };
                Ok(())
            })();
            writeln!(
                out,
                "result {} {calls}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            all(&mut out, &arena, &slots)?;
        }
    }
    Ok(())
}
