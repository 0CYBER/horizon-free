//! API engine point validation. No source or listed-call-site fallback exists.
use super::internals::Catalog;
use crate::i18n::trf;
use horizon_mod_package::{manifest::inject::Kind, Manifest};

pub(super) fn check(catalog: &Catalog, manifest: &Manifest) -> Result<(), String> {
    for entry in &manifest.inject {
        let id = format!("{}:{}", manifest.id, entry.id);
        let point = catalog
            .points
            .iter()
            .find(|point| {
                super::internals::plain_path(&point.target)
                    == super::internals::plain_path(&entry.at)
            })
            .ok_or_else(|| {
                trf(
                    "E_INJECT_MISSING: {id}: {at} has no API point",
                    &[("id", &id), ("at", &entry.at)],
                )
            })?;
        let target = catalog.target(&entry.at).ok_or_else(|| {
            trf(
                "E_INJECT_MISSING: {id}: {at} has no signature",
                &[("id", &id), ("at", &entry.at)],
            )
        })?;
        if let Some(hash) = entry.hash.as_ref().filter(|hash| *hash != &target.hash) {
            return Err(trf("E_INJECT_CHANGED: {id}: {at}: pinned {pinned}, installed {installed} ({signature})", &[("id", &id), ("at", &entry.at), ("pinned", hash), ("installed", &target.hash), ("signature", &target.signature)]));
        }
        if let Some(reason) = &point.api_refusal {
            return Err(trf(
                "{id}: no API listener at {at}: {reason}",
                &[("id", &id), ("at", &entry.at), ("reason", reason)],
            ));
        }
        if entry.kind == Kind::Return && !point.at_return {
            return Err(trf(
                "{id}: no API return point at {at}",
                &[("id", &id), ("at", &entry.at)],
            ));
        }
        if entry.kind == Kind::Replace {
            horizon_mod_point_emit::replaceable(&id, &entry.at, &point.emitted())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
