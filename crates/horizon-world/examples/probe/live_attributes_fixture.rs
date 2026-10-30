use crate::support::common::f64_hex;
use horizon_world::{
    attribute_map::ValueKind,
    attributes::{
        Modifier, Operation,
        entity::Catalog,
        live::{Error, Handle, LiveAttributes},
    },
    effect_attribute_modifiers::{self as effects, Definitions, Template},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn error(e: Error) -> &'static str {
    match e {
        Error::NullAttribute | Error::MissingInstanceModifier => "null",
        Error::MissingAttribute => "attribute",
        Error::MissingModifier => "modifier",
        Error::DuplicateModifier(_) => "duplicate",
        _ => panic!("invalid fixture: {e:?}"),
    }
}
fn handle(h: Option<&Handle>) -> String {
    h.map_or("null".into(), |h| h.index().to_string())
}
fn refs(mods: &[Modifier], pool: &mut Vec<Modifier>) -> String {
    mods.iter()
        .map(|m| {
            let id = pool
                .iter()
                .position(|p| p.same_identity(m))
                .unwrap_or_else(|| {
                    pool.push(m.clone());
                    pool.len() - 1
                });
            format!(
                "{id}:{}:{}:{}",
                m.id(),
                f64_hex(m.amount()),
                m.operation() as u8
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn effect_error(e: effects::Error<Error>) -> Error {
    match e {
        effects::Error::World(e) => e,
        _ => panic!("fixture definitions unexpectedly invalid"),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let dir = Path::new(&args[1]);
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    let mut out = io::BufWriter::new(fs::File::create(dir.join(format!("rust-{}.txt", args[2])))?);
    for line in fs::read_to_string(dir.join(format!("inputs-{}.txt", args[2])))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let hashes: Vec<_> = f["holder_hashes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect();
        let mut identities = f["instance_hashes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32);
        let mut attrs = LiveAttributes::new(
            catalog.clone(),
            f["entity"].as_str().unwrap(),
            &hashes,
            || identities.next().unwrap(),
        )
        .unwrap();
        let mut defs = Definitions::default();
        let mut pool = Vec::new();
        for op in f["ops"].as_array().unwrap() {
            let index = op["attribute"].as_i64().unwrap();
            let key = (index >= 0).then(|| attrs.keys()[index as usize].clone());
            let m = Modifier::new(
                op["id"].as_str().unwrap(),
                number(&op["amount"]),
                Operation::ALL[op["operation"].as_u64().unwrap() as usize],
            )?;
            let mode = op["mode"].as_str().unwrap();
            let result: Result<String, Error> = match mode {
                "value" => attrs.value(key, ValueKind::Computed).map(f64_hex),
                "base" => attrs.value(key, ValueKind::Base).map(f64_hex),
                "has" => attrs.has_attribute(key).map(|v| v.to_string()),
                "has_modifier" => attrs.has_modifier(key, m.id()).map(|v| v.to_string()),
                "modifier" => attrs.modifier_value(key, m.id()).map(f64_hex),
                "ensure" => attrs.get_instance(key).map(|h| handle(h.as_ref())),
                "reset" => attrs.reset_base(key).map(|v| v.to_string()),
                "ack_update" => {
                    let h = attrs.lookup(key.as_ref());
                    attrs.updates_mut().remove(h.as_ref());
                    Ok("ok".into())
                }
                "ack_sync" => {
                    let h = attrs.lookup(key.as_ref());
                    attrs.sync_mut().remove(h.as_ref());
                    Ok("ok".into())
                }
                "clear_updates" => {
                    attrs.updates_mut().clear();
                    Ok("ok".into())
                }
                "clear_sync" => {
                    attrs.sync_mut().clear();
                    Ok("ok".into())
                }
                "define" => {
                    effects::add_definition(
                        &mut defs,
                        key,
                        Template::new(m.id(), m.amount(), m.operation())?,
                    );
                    Ok("ok".into())
                }
                "effect-add" => attrs
                    .add_effect_modifiers(&mut defs, op["amplifier"].as_i64().unwrap() as i32)
                    .map_err(effect_error)
                    .map(|()| "ok".into()),
                "effect-remove" => attrs
                    .remove_effect_modifiers(&mut defs)
                    .map_err(effect_error)
                    .map(|()| "ok".into()),
                _ => match attrs.get_instance(key) {
                    Err(e) => Err(e),
                    Ok(None) => Ok("absent".into()),
                    Ok(Some(h)) => attrs.edit(&h, |a, o| match mode {
                        "set" => a
                            .set_base_value(number(&op["base"]), o)
                            .map(|()| "ok".into()),
                        "add" => a.add_transient(m.clone(), o).map(|()| "ok".into()),
                        "update" => a
                            .add_or_update_transient(m.clone(), o)
                            .map(|()| "ok".into()),
                        "permanent" => a.add_permanent(m.clone(), o).map(|()| "ok".into()),
                        "remove" => a.remove(m.id(), o).map(|v| v.to_string()),
                        "clear" => a.remove_all(o).map(|()| "ok".into()),
                        _ => panic!("mode"),
                    }),
                },
            };
            writeln!(
                out,
                "result {}",
                result.unwrap_or_else(|e| format!("error {}", error(e)))
            )?;
            writeln!(
                out,
                "map {}",
                attrs
                    .entries()
                    .iter()
                    .map(|(k, h)| format!(
                        "{}={}",
                        k.as_ref().map_or("null", |k| k.name()),
                        handle(h.as_ref())
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
            writeln!(
                out,
                "updates {}",
                attrs
                    .updates()
                    .values()
                    .map(handle)
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
            writeln!(
                out,
                "sync {}",
                attrs
                    .sync()
                    .values()
                    .map(handle)
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
            for (h, key, a) in attrs.instances() {
                writeln!(
                    out,
                    "instance {} {} {} {} {} | {} | {}",
                    h.index(),
                    key.name(),
                    f64_hex(a.base_value()),
                    a.is_dirty(),
                    f64_hex(a.cached_value()),
                    refs(a.modifiers(), &mut pool),
                    refs(a.permanent_modifiers(), &mut pool)
                )?;
            }
        }
    }
    Ok(())
}
