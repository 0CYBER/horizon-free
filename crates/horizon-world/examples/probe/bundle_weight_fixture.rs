use horizon_world::{
    bundle_weight::{self, BundleWeight, ContentsError, Fraction, ItemWeights},
    component_patch::Prototype,
    item_stack_storage::ItemStackData,
    item_template::{self, ValidationRules},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone)]
struct Item {
    kind: usize,
    count: i32,
    maximum: Option<i32>,
    bees: bool,
    nested: Option<BundleWeight<Item>>,
}
struct View([Option<i32>; 2]);
impl ItemWeights<Item> for View {
    fn count(&self, item: &Item) -> i32 {
        item.count
    }
    fn max_stack_size(&self, item: &Item) -> i32 {
        item.maximum.or(self.0[item.kind]).unwrap_or(1)
    }
    fn has_bees(&self, item: &Item) -> bool {
        item.bees
    }
    fn bundle(&self, item: &Item) -> Option<BundleWeight<Item>> {
        item.nested.clone()
    }
}
struct Rules<'a> {
    view: &'a View,
    container: Option<&'a [Item]>,
    bundle: Option<&'a BundleWeight<Item>>,
    projectiles: Option<&'a [Item]>,
}
impl ValidationRules<u8, u8, i32> for Rules<'_> {
    type ContentsError = ContentsError;
    fn has_max_damage(&self, s: &ItemStackData<u8, u8, i32>) -> bool {
        s.component(&1).is_some()
    }
    fn max_stack_size(&self, s: &ItemStackData<u8, u8, i32>) -> i32 {
        s.component(&0).copied().unwrap_or(1)
    }
    fn validate_contents(&self, s: &ItemStackData<u8, u8, i32>) -> Result<(), ContentsError> {
        if s.is_empty() {
            Ok(())
        } else {
            bundle_weight::validate_contents(
                self.container,
                self.bundle,
                self.projectiles,
                self.view,
            )
        }
    }
}
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn maximum(v: &Value) -> Option<i32> {
    v.as_i64().map(|x| x as i32)
}
fn fraction(f: Fraction) -> String {
    format!("{}/{}", f.numerator(), f.denominator())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        if f["mode"] == "fraction" {
            match (
                Fraction::new(n(&f, "a"), n(&f, "b")),
                Fraction::new(n(&f, "c"), n(&f, "d")),
            ) {
                (Ok(a), Ok(b)) => {
                    writeln!(out, "new {} {}", fraction(a), fraction(b))?;
                    writeln!(
                        out,
                        "add {}",
                        a.checked_add(b).map_or("error".into(), fraction)
                    )?;
                    writeln!(
                        out,
                        "multiply {}",
                        a.checked_multiply(b).map_or("error".into(), fraction)
                    )?;
                }
                _ => writeln!(out, "new error")?,
            }
            continue;
        }
        let mut view = View(std::array::from_fn(|i| maximum(&f["prototypes"][i])));
        let mut bundles: Vec<BundleWeight<Item>> = Vec::new();
        for b in f["bundles"].as_array().unwrap() {
            let items = b
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Item {
                    kind: n(v, "kind") as usize,
                    count: n(v, "count"),
                    maximum: maximum(&v["maximum"]),
                    bees: v["bees"].as_bool().unwrap(),
                    nested: usize::try_from(n(v, "nested"))
                        .ok()
                        .map(|i| bundles[i].clone()),
                })
                .collect();
            bundles.push(BundleWeight::new(items));
        }
        for op in f["ops"].as_array().unwrap() {
            match op["kind"].as_str().unwrap() {
                "prototype" => {
                    view.0[n(op, "index") as usize] = maximum(&op["maximum"]);
                    writeln!(out, "prototype")?;
                }
                "weight" => writeln!(
                    out,
                    "weight {}",
                    bundles[n(op, "index") as usize]
                        .weight(&view)
                        .map_or("error".into(), fraction)
                )?,
                "validate" => {
                    let mut s = ItemStackData::new(1, 0, n(op, "count"), Prototype::new([]));
                    if let Some(m) = maximum(&op["maximum"]) {
                        s.components.set(0, Some(m))?;
                    }
                    if op["damage"].as_bool().unwrap() {
                        s.components.set(1, Some(0))?;
                    }
                    let get = |k| usize::try_from(n(op, k)).ok().map(|i| &bundles[i]);
                    let rules = Rules {
                        view: &view,
                        container: get("container").map(BundleWeight::items),
                        bundle: get("bundle"),
                        projectiles: get("projectiles").map(BundleWeight::items),
                    };
                    writeln!(
                        out,
                        "valid {}",
                        item_template::validate_strict(&s, &rules).is_ok()
                    )?;
                }
                _ => panic!(),
            }
        }
    }
    Ok(())
}
