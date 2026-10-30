use horizon_world::{
    component_patch::{Patch, Prototype},
    item_stack_refs::{ItemStacks, StackRef},
    item_stack_storage::ItemStackData,
    item_template::{self, ItemTemplate, ValidationRules},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = StackRef<u8, usize, i32>;
type Template = ItemTemplate<u8, usize, i32>;
struct Rules;
impl ValidationRules<u8, usize, i32> for Rules {
    type ContentsError = std::convert::Infallible;
    fn has_max_damage(&self, s: &ItemStackData<u8, usize, i32>) -> bool {
        s.component(&1).is_some()
    }
    fn max_stack_size(&self, s: &ItemStackData<u8, usize, i32>) -> i32 {
        s.component(&0).copied().unwrap_or(1)
    }
    fn validate_contents(
        &self,
        _: &ItemStackData<u8, usize, i32>,
    ) -> Result<(), Self::ContentsError> {
        Ok(())
    }
}
fn opt(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
}
fn patch(v: &Value) -> Patch<usize, i32> {
    Patch::new(v.as_array().unwrap().iter().map(|v| {
        (
            v[0].as_u64().unwrap() as usize,
            v[1].as_i64().map(|n| n as i32),
        )
    }))
}
fn proto(v: &Value) -> Prototype<usize, i32> {
    Prototype::new(v.as_array().unwrap().iter().map(|v| {
        (
            v[0].as_u64().unwrap() as usize,
            v[1].as_i64().unwrap() as i32,
        )
    }))
}
fn patch_text(p: &Patch<usize, i32>) -> String {
    p.entries()
        .iter()
        .map(|(k, v)| format!("{k}={}", opt(*v)))
        .collect::<Vec<_>>()
        .join(",")
}
fn all(
    out: &mut dyn Write,
    stacks: &[Stack],
    templates: &[Option<Template>],
    prototypes: &[Prototype<usize, i32>],
    store: &ItemStacks<u8, usize, i32>,
) -> io::Result<()> {
    for (i, s) in stacks.iter().enumerate() {
        let data = s.read();
        let values = (0..3)
            .map(|k| opt(data.component(&k).copied()))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "stack {i} {} {} {} {} {} {} {} | {} | {}",
            s.same_reference(&store.empty()),
            data.item(),
            data.count,
            data.count(),
            data.pop_time,
            data.is_empty(),
            item_template::validate_strict(&data, &Rules).is_ok(),
            values,
            patch_text(&data.components.as_patch())
        )?;
    }
    for (i, t) in templates.iter().enumerate() {
        if let Some(t) = t {
            let values = (0..3)
                .map(|k| opt(t.component(&prototypes[t.item() as usize], &k).copied()))
                .collect::<Vec<_>>()
                .join(",");
            writeln!(
                out,
                "template {i} {} {} | {} | {}",
                t.item(),
                t.count(),
                values,
                patch_text(t.patch())
            )?;
        } else {
            writeln!(out, "template {i} none")?;
        }
    }
    for a in 0..3 {
        for b in a + 1..3 {
            let same = match (&templates[a], &templates[b]) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_reference(b),
                _ => false,
            };
            writeln!(
                out,
                "same {a} {b} {} {same}",
                stacks[a].same_reference(&stacks[b])
            )?;
        }
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let store = ItemStacks::new(0);
        let mut prototypes: Vec<_> = f["prototypes"]
            .as_array()
            .unwrap()
            .iter()
            .map(proto)
            .collect();
        let mut stacks: Vec<_> = f["stacks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let id = v["item"].as_i64().unwrap();
                if id < 0 {
                    store.empty()
                } else {
                    let s = store.create(
                        id as u8,
                        v["count"].as_i64().unwrap() as i32,
                        prototypes[id as usize].clone(),
                    );
                    s.write().pop_time = 5;
                    s.write().components.restore_patch(&patch(&v["patch"]));
                    s
                }
            })
            .collect();
        let mut templates: Vec<Option<Template>> = vec![None, None, None];
        all(&mut out, &stacks, &templates, &prototypes, &store)?;
        for op in f["ops"].as_array().unwrap() {
            let a = op["a"].as_u64().unwrap() as usize;
            let b = op["b"].as_u64().unwrap() as usize;
            let n = op["count"].as_i64().unwrap() as i32;
            let result: Result<(), ()> = (|| {
                match op["kind"].as_str().unwrap() {
                    "new" => {
                        templates[b] = Some(
                            Template::new(
                                op["item"].as_u64().unwrap() as u8,
                                0,
                                n,
                                patch(&op["patch"]),
                            )
                            .map_err(|_| ())?,
                        )
                    }
                    "from" => {
                        templates[b] =
                            Some(Template::from_stack(&stacks[a].read(), 0).map_err(|_| ())?)
                    }
                    "from-nonempty" => {
                        templates[b] = Some(
                            Template::from_non_empty_stack(&stacks[a].read(), 0).map_err(|_| ())?,
                        )
                    }
                    "from-count" => {
                        templates[b] = Some(
                            Template::from_non_empty_stack_with_count(&stacks[a].read(), 0, n)
                                .map_err(|_| ())?,
                        )
                    }
                    "with" => {
                        templates[b] = Some(
                            templates[a]
                                .as_ref()
                                .ok_or(())?
                                .with_count(n)
                                .map_err(|_| ())?,
                        )
                    }
                    "create" => {
                        let t = templates[a].as_ref().ok_or(())?;
                        stacks[b] = t
                            .create(&store, prototypes[t.item() as usize].clone(), &Rules)
                            .0;
                    }
                    "apply" | "apply-count" => {
                        let t = templates[a].as_ref().ok_or(())?;
                        let count = if op["kind"] == "apply" { t.count() } else { n };
                        stacks[b] = t
                            .apply(
                                &store,
                                prototypes[t.item() as usize].clone(),
                                count,
                                &patch(&op["patch"]),
                                &Rules,
                            )
                            .0;
                    }
                    "validate-patch" => {
                        let _ = item_template::apply_validated(
                            &mut stacks[a].write(),
                            &patch(&op["patch"]),
                            &Rules,
                        );
                    }
                    "count" => stacks[a].write().count = n,
                    "prototype" => {
                        prototypes[op["item"].as_u64().unwrap() as usize] = proto(&op["prototype"])
                    }
                    _ => panic!(),
                }
                Ok(())
            })();
            writeln!(
                out,
                "result {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            all(&mut out, &stacks, &templates, &prototypes, &store)?;
        }
    }
    Ok(())
}
