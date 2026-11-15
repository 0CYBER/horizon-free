//! Tool-written native inject declarations, pinned to the game's metadata.
use horizon_mod_package::{
    Manifest,
    internals::ListedInternal,
    manifest::{
        MANIFEST,
        inject::{InjectError, Kind},
    },
};
use std::{fs, path::Path};
use toml_edit::{DocumentMut, Table, value};

const USAGE: &str = "usage: cargo horizon-mod inject add <at> --kind <kind> [--with <fn>]";

#[derive(serde::Deserialize)]
struct Catalog {
    #[serde(default)]
    points: Vec<Point>,
}
/// A function's point, as `points.json` lists it: whether its API listeners have a record.
#[derive(serde::Deserialize)]
struct Point {
    target: String,
    #[serde(default)]
    api_refusal: Option<String>,
}
struct Request<'a> {
    at: &'a str,
    kind_name: &'a str,
    with: &'a str,
}

fn request(args: &[String]) -> Result<Request<'_>, String> {
    let [command, at, rest @ ..] = args else {
        return Err(USAGE.into());
    };
    if command != "add" || at.starts_with('-') {
        return Err(USAGE.into());
    }
    let (mut kind, mut with) = (None, None);
    for pair in rest.chunks(2) {
        let [flag, val] = pair else {
            return Err(USAGE.into());
        };
        if val.starts_with('-') {
            return Err(USAGE.into());
        }
        match flag.as_str() {
            "--kind" if kind.is_none() => kind = Some(val.as_str()),
            "--with" if with.is_none() => with = Some(val.as_str()),
            _ => return Err(USAGE.into()),
        }
    }
    let kind_name = kind.ok_or(USAGE)?;
    let kind: Kind = serde_json::from_value(serde_json::Value::String(kind_name.into()))
        .map_err(|e| format!("invalid inject kind: {e}"))?;
    Ok(Request {
        at,
        kind_name: kind.name(),
        with: with.unwrap_or("on_inject"),
    })
}

#[path = "inject_update.rs"]
pub(crate) mod update;

pub fn run(args: &[String]) -> Result<(), String> {
    if args.first().is_some_and(|command| command == "update") {
        return update::run(Path::new("."), &args[1..]);
    }
    if args.first().is_some_and(|command| command == "sync") {
        return crate::inject_sync::run(&args[1..]);
    }
    add(Path::new("."), args)
}

fn add(dir: &Path, args: &[String]) -> Result<(), String> {
    let request = request(args)?;
    let path = dir.join(MANIFEST);
    let original = crate::bounded::read(&path, crate::bounded::MAX_MANIFEST)?;
    // Manifest::parse owns mod id, native pin and existing declaration validation.
    let manifest = Manifest::parse(&original)?;
    let directory = crate::internals::catalog_directory(dir, &manifest)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("inject add needs a [native] mod")?;
    let internals = if native.module.is_empty() {
        crate::internals::load(dir, &native.horizon)?
    } else {
        horizon_mod_package::internals::read(&directory.join("internals.json"))?
    };
    let points = directory.join("points.json");
    let (_, catalog): (_, Catalog) = horizon_mod_package::build_data::read(
        horizon_mod_package::build_data::POINTS,
        &crate::bounded::read(&points, crate::bounded::MAX_CATALOG)?,
    )
    .map_err(|e| format!("{}: {e}", points.display()))?;
    let candidate = append(&original, &manifest, &request, &internals, &catalog)?;
    crate::internals::atomic_write_manifest(&path, candidate)?;
    println!("{}", availability(&catalog, &request));
    Ok(())
}

/// Whether the requested engine function has an API record.
fn availability(catalog: &Catalog, request: &Request<'_>) -> String {
    use horizon_mod_point_emit::plain_path;
    let api = match catalog
        .points
        .iter()
        .find(|point| plain_path(&point.target) == plain_path(request.at))
    {
        Some(Point {
            api_refusal: None, ..
        }) => "available (native and wasm)".to_owned(),
        Some(Point {
            api_refusal: Some(why),
            ..
        }) => format!("refused, {why}"),
        None => format!("refused, {} is not a point", request.at),
    };
    format!("api: {api}")
}

fn append(
    text: &str,
    manifest: &Manifest,
    request: &Request<'_>,
    internals: &[ListedInternal],
    catalog: &Catalog,
) -> Result<String, String> {
    let missing = || {
        InjectError::NoSlot {
            id: manifest.id.clone(),
            reason: horizon_mod_package::manifest::inject::NoSlotReason::NoApiRecord,
            declaration: request.at.into(),
        }
        .to_string()
    };
    let item = internals
        .iter()
        .find(|i| {
            i.path == request.at && !i.shape_only && matches!(i.kind.as_str(), "fn" | "method")
        })
        .ok_or_else(missing)?;
    let item = horizon_mod_package::internals::resolve(
        internals,
        &manifest.id,
        request.at,
        Some(&item.kind),
    )
    .map_err(|e| e.to_string())?;
    if !catalog
        .points
        .iter()
        .any(|point| point.target == request.at)
    {
        return Err(missing());
    }
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    if doc.get("inject").is_none() {
        doc["inject"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let array = doc["inject"]
        .as_array_of_tables_mut()
        .ok_or("inject add requires [[inject]] tables")?;
    let mut table = Table::new();
    table["id"] = value(request.with);
    table["at"] = value(&item.path);
    table["kind"] = value(request.kind_name);
    table["with"] = value(request.with);
    table["hash"] = value(&item.hash);
    array.push(table);
    let result = doc.to_string();
    Manifest::parse(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEXT: &str = "id='example'\nname='Example'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nmixins='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'\nhorizon='0123456789012345678901234567890123456789'\n";
    #[test]
    fn add_pins_api_point_and_preserves_existing_text() {
        let manifest = Manifest::parse(TEXT).unwrap();
        let internals: Vec<ListedInternal> = serde_json::from_str(
            r#"[{"path":"game::tick","kind":"fn","signature":"fn tick()","hash":"signature"}]"#,
        )
        .unwrap();
        let catalog: Catalog =
            serde_json::from_str(r#"{"points":[{"target":"game::tick"}]}"#).unwrap();
        let args: Vec<String> = ["add", "game::tick", "--kind", "head"]
            .map(String::from)
            .into();
        let request = request(&args).unwrap();
        let result = append(TEXT, &manifest, &request, &internals, &catalog).unwrap();
        assert!(result.starts_with(TEXT));
        let parsed = Manifest::parse(&result).unwrap();
        assert_eq!(parsed.inject[0].hash.as_deref(), Some("signature"));
    }
    #[test]
    fn call_flags_are_refused() {
        let args: Vec<String> = ["add", "game::tick", "--kind", "head", "--nth", "0"]
            .map(String::from)
            .into();
        assert!(request(&args).is_err());
    }
}
