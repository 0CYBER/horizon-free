//! Transactional freshness updates; never changes the game pin.
use super::*;

pub(super) fn run(dir: &Path, args: &[String]) -> Result<(), String> {
    let mut accept = Vec::new();
    for pair in args.chunks(2) {
        match pair {
            [flag, id] if flag == "--accept" => accept.push(id.as_str()),
            _ => return Err("usage: cargo horizon-mod inject update [--accept <id>]…".into()),
        }
    }
    let path = dir.join(MANIFEST);
    let original = crate::bounded::read(&path, crate::bounded::MAX_MANIFEST)?;
    let manifest = Manifest::parse(&original)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("inject update needs a [native] mod")?;
    let internals = crate::internals::load(dir, &native.horizon)?;
    let points = crate::sdk_fetch::build_data(dir, &native.horizon)?.join("points.json");
    let (_, catalog): (_, Catalog) = horizon_mod_package::build_data::read(
        horizon_mod_package::build_data::POINTS,
        &crate::bounded::read(&points, crate::bounded::MAX_CATALOG)?,
    )?;
    // Other builds' cached build data provides the signature behind a declaration's old hash.
    let mut old = Vec::new();
    let cache = dir.join(crate::sdk_fetch::CACHE);
    for entry in fs::read_dir(&cache).map_err(|e| format!("{}: {e}", cache.display()))? {
        let path = entry
            .map_err(|e| e.to_string())?
            .path()
            .join(crate::sdk_fetch::BUILD_DATA)
            .join("internals.json");
        let text = match crate::bounded::read(&path, crate::bounded::MAX_CATALOG) {
            Ok(text) => Some(text),
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(text) = text
            && let Ok(snapshot) = horizon_mod_package::internals::parse(&text)
        {
            old.extend(snapshot);
        }
    }
    let (candidate, reports, refused) =
        plan(&original, &manifest, &internals, &catalog, &old, &accept)?;
    for report in reports {
        println!("{report}");
    }
    if refused {
        return Err("inject update refused; no files changed".into());
    }
    if candidate != original {
        crate::internals::atomic_write_manifest(&path, candidate)?;
    }
    Ok(())
}

/// The catalog entry an inject targets: a function or method, or a struct for a field.
fn target<'a>(
    internals: &'a [ListedInternal],
    inject: &horizon_mod_package::manifest::inject::Inject,
) -> Option<&'a ListedInternal> {
    let kind = "fn";
    internals.iter().find(|i| {
        i.path == inject.at
            && !i.shape_only
            && (i.kind == kind || (kind == "fn" && i.kind == "method"))
    })
}

/// What moving the pin from the `old` catalog to the `new` one does to the injects that carry
/// no hash, which follow the pin: a changed signature is refused as `E_INJECT_CHANGED` unless
/// its id is in `accept`, exactly as a stale pinned hash is refused at build, and a target the
/// new game lacks is `E_INJECT_MISSING`. Returns the reports and whether to refuse.
pub(crate) fn pin_moves(
    manifest: &Manifest,
    old: &[ListedInternal],
    new: &[ListedInternal],
    accept: &[&str],
) -> (Vec<String>, bool) {
    let mut reports = Vec::new();
    let mut refused = false;
    for inject in manifest
        .inject
        .iter()
        .filter(|inject| inject.hash.is_none())
    {
        let id = format!("{}:{}", manifest.id, inject.id);
        let Some(now) = target(new, inject) else {
            reports.push(
                InjectError::Missing {
                    id,
                    at: inject.at.clone(),
                }
                .to_string(),
            );
            refused = true;
            continue;
        };
        let before = target(old, inject);
        if before.is_some_and(|before| before.hash == now.hash) {
            continue;
        }
        reports.push(
            InjectError::Changed {
                id,
                at: inject.at.clone(),
                old: before.map_or_else(
                    || "absent from the old game".to_owned(),
                    |before| format!("{} ({})", before.hash, before.signature),
                ),
                new: format!("{} ({})", now.hash, now.signature),
            }
            .to_string(),
        );
        refused |= !accept.contains(&inject.id.as_str());
    }
    (reports, refused)
}

fn plan(
    text: &str,
    manifest: &Manifest,
    internals: &[ListedInternal],
    _catalog: &Catalog,
    old: &[ListedInternal],
    accept: &[&str],
) -> Result<(String, Vec<String>, bool), String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    let mut reports = Vec::new();
    let mut refused = false;
    let mut edited = false;
    for id in accept {
        if !manifest.inject.iter().any(|i| i.id == *id) {
            reports.push(format!("unknown inject id {id}"));
            refused = true;
        }
    }
    for (index, inject) in manifest.inject.iter().enumerate() {
        let id = format!("{}:{}", manifest.id, inject.id);
        let Some(item) = target(internals, inject) else {
            reports.push(
                InjectError::Missing {
                    id,
                    at: inject.at.clone(),
                }
                .to_string(),
            );
            refused = true;
            continue;
        };
        let item = match horizon_mod_package::internals::resolve(
            internals,
            &id,
            &inject.at,
            Some(&item.kind),
        ) {
            Ok(item) => item,
            Err(error) => {
                reports.push(error.to_string());
                refused = true;
                continue;
            }
        };
        // A declaration without a hash follows the pin: there is nothing of its own to refresh.
        let pinned = inject.hash.as_deref();
        let changed = pinned.is_some_and(|hash| hash != item.hash);
        if let Some(hash) = pinned.filter(|_| changed) {
            let previous = old
                .iter()
                .find(|i| i.path == inject.at && i.hash == hash)
                .map(|i| i.signature.as_str())
                .unwrap_or("signature unavailable in cached snapshots");
            reports.push(
                InjectError::Changed {
                    id,
                    at: inject.at.clone(),
                    old: format!("{hash} ({previous})"),
                    new: format!("{} ({})", item.hash, item.signature),
                }
                .to_string(),
            );
            if !accept.contains(&inject.id.as_str()) {
                refused = true;
                continue;
            }
        }
        let table = doc
            .get_mut("inject")
            .and_then(|i| i.as_array_of_tables_mut())
            .and_then(|a| a.get_mut(index))
            .ok_or("missing inject table")?;
        // Replacing the scalar payload retains its existing comments and spacing.
        {
            let key = "hash";
            let replacement = changed.then(|| toml_edit::Value::from(item.hash.as_str()));
            if let Some(mut replacement) = replacement {
                edited = true;
                if let Some(existing) = table.get(key).and_then(|i| i.as_value()) {
                    *replacement.decor_mut() = existing.decor().clone();
                }
                table[key] = toml_edit::Item::Value(replacement);
            }
        }
    }
    let candidate = if refused || !edited {
        text.to_owned()
    } else if text.contains("\r\n") {
        doc.to_string().replace('\n', "\r\n")
    } else {
        doc.to_string()
    };
    Manifest::parse(&candidate)?;
    Ok((candidate, reports, refused))
}

pub(crate) fn sync(dir: &Path, repo: &Path) -> Result<(), String> {
    let manifest = Manifest::parse(&crate::bounded::read(
        &dir.join(MANIFEST),
        crate::bounded::MAX_MANIFEST,
    )?)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("inject sync needs a [native] mod")?;
    let root = repo
        .join("target/horizon-mod/prebuilt")
        .join(&native.horizon);
    let (_, points): (_, Catalog) = horizon_mod_package::build_data::read(
        horizon_mod_package::build_data::POINTS,
        &crate::bounded::read(&root.join("points.json"), crate::bounded::MAX_CATALOG)?,
    )?;
    let internals = horizon_mod_package::internals::parse(&crate::bounded::read(
        &root.join("internals.json"),
        crate::bounded::MAX_CATALOG,
    )?)?;
    for inject in &manifest.inject {
        let item =
            target(&internals, inject).ok_or_else(|| format!("E_INJECT_MISSING: {}", inject.at))?;
        if inject.hash.as_ref().is_some_and(|hash| hash != &item.hash) {
            return Err(format!("E_INJECT_CHANGED: {}", inject.at));
        }
        let point = points
            .points
            .iter()
            .find(|p| p.target == inject.at)
            .ok_or_else(|| format!("E_INJECT_MISSING: {}", inject.at))?;
        if let Some(reason) = &point.api_refusal {
            return Err(format!("E_INJECT_NO_SLOT: {}: {reason}", inject.at));
        }
    }
    Ok(())
}
