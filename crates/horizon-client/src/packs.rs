//! Resource packs (PackRepository / FolderRepositorySource): discovery of
//! folder and zip packs in `resourcepacks/` plus the built-in Programmer Art
//! and High Contrast packs, `pack.mcmeta` metadata and format compatibility,
//! and an overlay resource root that layers the enabled packs over the
//! vanilla resources (later packs win), so every loader sees one tree.
mod pattern;
use crate::{text::Text, zip::Zip};
use pattern::Pattern;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

/// SharedConstants resource pack format of 26.3 (major, minor).
pub const RESOURCE_FORMAT: (i64, i64) = (97, 1);

#[derive(Clone, Debug)]
pub enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
}
#[derive(Clone, Debug)]
pub struct Pack {
    /// `file/<name>` for user packs, the bare name for built-in ones.
    pub id: String,
    pub source: Source,
    pub title: Text,
    pub description: Text,
    pub compatible: bool,
    /// Overlay directories active for this game version, in `pack.mcmeta` order.
    pub overlays: Vec<String>,
    /// `filter.block` entries (namespace and path patterns) hiding resources
    /// of the packs below this one.
    pub filter: Vec<Filter>,
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    namespace: Option<Pattern>,
    path: Option<Pattern>,
}
impl Filter {
    fn blocks(&self, file: &str) -> bool {
        let Some(rest) = file.strip_prefix("assets/") else {
            return false;
        };
        let Some((namespace, path)) = rest.split_once('/') else {
            return false;
        };
        self.namespace
            .as_ref()
            .is_none_or(|p| p.is_match(namespace))
            && self.path.as_ref().is_none_or(|p| p.is_match(path))
    }
}
/// A minimal text component from JSON (strings, `text`, `translate`, arrays).
pub fn component(v: &serde_json::Value) -> Text {
    match v {
        serde_json::Value::String(s) => Text::legacy(s),
        serde_json::Value::Array(a) => {
            let mut t = Text::empty();
            for e in a {
                t.push(component(e));
            }
            t
        }
        serde_json::Value::Object(o) => {
            let mut t = if let Some(s) = o.get("text").and_then(|s| s.as_str()) {
                Text::legacy(s)
            } else if let Some(k) = o.get("translate").and_then(|s| s.as_str()) {
                let args: Vec<Text> = o
                    .get("with")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().map(component).collect())
                    .unwrap_or_default();
                Text::translate(k, &args)
            } else {
                Text::empty()
            };
            if let Some(extra) = o.get("extra") {
                t.push(component(extra));
            }
            if let Some(c) = o.get("color").and_then(|c| c.as_str()) {
                let rgb = crate::text::named_color(c).or_else(|| {
                    c.strip_prefix('#')
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                });
                if let Some(rgb) = rgb {
                    t = t.with_color(rgb);
                }
            }
            t
        }
        serde_json::Value::Number(n) => Text::literal(n.to_string()),
        serde_json::Value::Bool(b) => Text::literal(b.to_string()),
        serde_json::Value::Null => Text::empty(),
    }
}
fn format_of(v: &serde_json::Value) -> Option<(i64, i64)> {
    match v {
        serde_json::Value::Number(n) => Some((n.as_i64()?, 0)),
        serde_json::Value::Array(a) => Some((
            a.first()?.as_i64()?,
            a.get(1).and_then(|m| m.as_i64()).unwrap_or(0),
        )),
        _ => None,
    }
}
/// PackCompatibility: inside [min_format, max_format] (or the legacy
/// `pack_format`/`supported_formats`).
pub fn compatible(meta: &serde_json::Value) -> bool {
    let pack = &meta["pack"];
    let current = RESOURCE_FORMAT;
    if let (Some(min), Some(max)) = (
        format_of(&pack["min_format"]),
        format_of(&pack["max_format"]),
    ) {
        return min <= current
            && current <= (max.0, max.1.max(if max.1 == 0 { i64::MAX } else { max.1 }));
    }
    if let Some(range) = pack["supported_formats"].as_array()
        && let (Some(lo), Some(hi)) = (
            range.first().and_then(|v| v.as_i64()),
            range.get(1).and_then(|v| v.as_i64()),
        )
    {
        return lo <= current.0 && current.0 <= hi;
    }
    format_of(&pack["pack_format"]).is_some_and(|f| f.0 == current.0)
}
/// Whether an `overlays.entries[]` item applies to the current format
/// (OverlayMetadataSection: `formats` as an int, `[min, max]` or
/// `{min_inclusive, max_inclusive}`, or `min_format`/`max_format`).
fn overlay_applies(entry: &serde_json::Value) -> bool {
    let current = RESOURCE_FORMAT;
    if let (Some(min), Some(max)) = (
        format_of(&entry["min_format"]),
        format_of(&entry["max_format"]),
    ) {
        return min <= current && current <= (max.0, if max.1 == 0 { i64::MAX } else { max.1 });
    }
    let formats = &entry["formats"];
    if let Some(n) = formats.as_i64() {
        return n == current.0;
    }
    if let Some(a) = formats.as_array() {
        return match (
            a.first().and_then(|v| v.as_i64()),
            a.get(1).and_then(|v| v.as_i64()),
        ) {
            (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
            _ => false,
        };
    }
    match (
        formats["min_inclusive"].as_i64(),
        formats["max_inclusive"].as_i64(),
    ) {
        (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
        _ => false,
    }
}
fn overlays_of(meta: &serde_json::Value) -> Vec<String> {
    meta["overlays"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| overlay_applies(e))
        .filter_map(|e| e["directory"].as_str().map(str::to_owned))
        .collect()
}
fn filter_of(meta: &serde_json::Value) -> Vec<Filter> {
    meta["filter"]["block"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| Filter {
            namespace: b["namespace"].as_str().and_then(Pattern::new),
            path: b["path"].as_str().and_then(Pattern::new),
        })
        .collect()
}
fn read_meta(source: &Source) -> Option<serde_json::Value> {
    let bytes = match source {
        Source::Dir(d) => fs::read(d.join("pack.mcmeta")).ok()?,
        Source::Zip(z) => Zip::open(z).ok()?.read("pack.mcmeta").ok()?,
    };
    serde_json::from_slice(&bytes).ok()
}
fn pack(id: String, title: Text, source: Source) -> Option<Pack> {
    let meta = read_meta(&source)?;
    Some(Pack {
        id,
        title,
        description: component(&meta["pack"]["description"]),
        compatible: compatible(&meta),
        overlays: overlays_of(&meta),
        filter: filter_of(&meta),
        source,
    })
}
/// Built-in and `resourcepacks/` packs (vanilla itself excluded), sorted like
/// the pack screen (built-ins first, then by name).
pub fn discover(resources: &Path, packs_dir: &Path) -> Vec<Pack> {
    let mut out = Vec::new();
    for (id, key) in [
        ("programmer_art", "resourcePack.programmer_art.name"),
        ("high_contrast", "resourcePack.high_contrast.name"),
    ] {
        let z = resources.join(format!("assets/minecraft/resourcepacks/{id}.zip"));
        if z.exists()
            && let Some(p) = pack(id.into(), Text::translate(key, &[]), Source::Zip(z))
        {
            out.push(p);
        }
    }
    let mut user: Vec<Pack> = fs::read_dir(packs_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let source = if path.is_dir() {
                Source::Dir(path)
            } else if name.ends_with(".zip") {
                Source::Zip(path)
            } else {
                return None;
            };
            pack(format!("file/{name}"), Text::literal(name), source)
        })
        .collect();
    user.sort_by(|a, b| a.id.cmp(&b.id));
    out.extend(user);
    out
}
/// A file a pack provides: its path (folder packs) and its name inside the pack.
type Provided = (Option<PathBuf>, String);
/// Files a pack provides under `assets/`, keyed by their resource name; active
/// overlay directories are applied after the base, in order, later wins.
fn pack_files(pack: &Pack) -> io::Result<BTreeMap<String, Provided>> {
    let mut out = BTreeMap::new();
    let roots = std::iter::once(String::new())
        .chain(pack.overlays.iter().map(|d| format!("{d}/")))
        .collect::<Vec<_>>();
    match &pack.source {
        Source::Dir(d) => {
            for root in &roots {
                let base = d.join(root.trim_end_matches('/'));
                let mut stack = vec![base.join("assets")];
                while let Some(dir) = stack.pop() {
                    let Ok(entries) = fs::read_dir(&dir) else {
                        continue;
                    };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if let Ok(rel) = p.strip_prefix(&base) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            out.insert(rel.clone(), (Some(p.clone()), format!("{root}{rel}")));
                        }
                    }
                }
            }
        }
        Source::Zip(z) => {
            let zip = Zip::open(z)?;
            for root in &roots {
                let prefix = format!("{root}assets/");
                for name in zip.names() {
                    if let Some(rest) = name.strip_prefix(&prefix) {
                        out.insert(format!("assets/{rest}"), (None, name.to_owned()));
                    }
                }
            }
        }
    }
    Ok(out)
}
#[cfg(unix)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}
#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    if target.is_dir() {
        Ok(())
    } else {
        fs::copy(target, at).map(|_| ())
    }
}
/// Build `out` as `vanilla` with the packs' files layered on top (packs in
/// load order, last wins). Untouched directories are symlinks into vanilla.
pub fn build_overlay(vanilla: &Path, packs: &[Pack], out: &Path) -> io::Result<()> {
    if out.exists() {
        fs::remove_dir_all(out)?;
    }
    fs::create_dir_all(out)?;
    // Final owner of every overridden file.
    let mut files: BTreeMap<String, (usize, Provided)> = BTreeMap::new();
    for (i, p) in packs.iter().enumerate() {
        // A pack's filter hides what the packs below it provide.
        files.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        for (name, provided) in pack_files(p)? {
            files.insert(name, (i, provided));
        }
    }
    // Vanilla sits below every pack, so filters hide its files too.
    let filters: Vec<&Filter> = packs.iter().flat_map(|p| &p.filter).collect();
    let touched_dirs: BTreeSet<String> = files
        .keys()
        .flat_map(|f| {
            let parts: Vec<&str> = f.split('/').collect();
            (1..parts.len()).map(move |n| parts[..n].join("/"))
        })
        .collect();
    let zips: Vec<Option<Zip>> = packs
        .iter()
        .map(|p| {
            if let Source::Zip(z) = &p.source {
                Zip::open(z).ok()
            } else {
                None
            }
        })
        .collect();
    fn mirror(
        rel: &str,
        vanilla: &Path,
        out: &Path,
        touched: &BTreeSet<String>,
        files: &BTreeMap<String, (usize, Provided)>,
        filters: &[&Filter],
        zips: &[Option<Zip>],
    ) -> io::Result<()> {
        let src = if rel.is_empty() {
            vanilla.to_path_buf()
        } else {
            vanilla.join(rel)
        };
        let dst = if rel.is_empty() {
            out.to_path_buf()
        } else {
            out.join(rel)
        };
        fs::create_dir_all(&dst)?;
        let mut names: BTreeSet<String> = BTreeSet::new();
        if let Ok(entries) = fs::read_dir(&src) {
            names.extend(
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned()),
            );
        }
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        for f in files.keys().filter_map(|f| f.strip_prefix(&prefix)) {
            names.insert(f.split('/').next().unwrap_or("").to_owned());
        }
        for name in names {
            let child = format!("{prefix}{name}");
            if let Some((owner, (path, inner))) = files.get(&child) {
                let target = out.join(&child);
                match path {
                    Some(p) => link(p, &target)?,
                    None => {
                        if let Some(Some(z)) = zips.get(*owner) {
                            fs::write(&target, z.read(inner)?)?;
                        }
                    }
                }
            } else if vanilla.join(&child).is_dir()
                && (touched.contains(&child) || !filters.is_empty())
            {
                mirror(&child, vanilla, out, touched, files, filters, zips)?;
            } else if touched.contains(&child) {
                mirror(&child, vanilla, out, touched, files, filters, zips)?;
            } else if filters.iter().any(|f| f.blocks(&child)) {
                // Hidden by a pack's filter.
            } else {
                link(&vanilla.join(&child), &out.join(&child))?;
            }
        }
        Ok(())
    }
    mirror("", vanilla, out, &touched_dirs, &files, &filters, &zips)
}
/// The packs to apply for an options.txt `resourcePacks` list (bottom first).
pub fn enabled(all: &[Pack], ids: &[String]) -> Vec<Pack> {
    ids.iter()
        .filter(|id| *id != "vanilla")
        .filter_map(|id| all.iter().find(|p| &p.id == id).cloned())
        .collect()
}
/// Resource root for a pack selection: vanilla itself when no pack is on.
pub fn resource_root(vanilla: &Path, packs: &[Pack], cache: &Path) -> PathBuf {
    if packs.is_empty() {
        return vanilla.to_path_buf();
    }
    let key: String = packs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let out = cache.join(format!("{hash:016x}"));
    match build_overlay(vanilla, packs, &out) {
        Ok(()) => out,
        Err(e) => {
            eprintln!("Horizon: resource packs not applied: {e}");
            vanilla.to_path_buf()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_ranges() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert!(compatible(&v(
            r#"{"pack":{"min_format":97,"max_format":98}}"#
        )));
        assert!(compatible(&v(
            r#"{"pack":{"min_format":[97,0],"max_format":[97,1]}}"#
        )));
        assert!(!compatible(&v(
            r#"{"pack":{"min_format":[97,2],"max_format":99}}"#
        )));
        assert!(compatible(&v(r#"{"pack":{"pack_format":97}}"#)));
        assert!(!compatible(&v(r#"{"pack":{"pack_format":34}}"#)));
        assert_eq!(
            component(&v(r#"["a",{"text":"b","color":"red"}]"#)).plain(),
            "ab"
        );
    }
    #[test]
    fn overlay_layers_packs_over_vanilla() {
        let base = std::env::temp_dir().join(format!("horizon-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(vanilla.join("assets/minecraft/lang")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"vanilla dirt",
        )
        .unwrap();
        fs::write(vanilla.join("assets/minecraft/lang/en_us.json"), b"{}").unwrap();
        fs::write(vanilla.join("facts.json"), b"[]").unwrap();
        let pack_dir = base.join("packs/Mine");
        fs::create_dir_all(pack_dir.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            pack_dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"mine","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/textures/block/stone.png"),
            b"pack",
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/Mine");
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/dirt.png")).unwrap(),
            b"vanilla dirt"
        );
        assert_eq!(
            fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
            b"{}"
        );
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"[]");
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn overlays_and_filters() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":[90,96],"directory":"old"},
            {"formats":{"min_inclusive":97,"max_inclusive":97},"directory":"new"},
            {"min_format":97,"max_format":[97,5],"directory":"also"},
            {"formats":50,"directory":"no"}]}}"#);
        assert_eq!(overlays_of(&meta), ["new", "also"]);
        let base = std::env::temp_dir().join(format!("horizon-packs-ov-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"v",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"v",
        )
        .unwrap();
        let dir = base.join("packs/P");
        fs::create_dir_all(dir.join("new/assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(dir.join("old/assets/minecraft/textures/block")).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"p","pack_format":97},
            "overlays":{"entries":[{"formats":97,"directory":"new"},{"formats":1,"directory":"old"}]},
            "filter":{"block":[{"path":"textures/block/dirt\\.png"}]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("new/assets/minecraft/textures/block/stone.png"),
            b"overlay",
        )
        .unwrap();
        fs::write(
            dir.join("old/assets/minecraft/textures/block/stone.png"),
            b"old",
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"overlay"
        );
        assert!(
            !out.join("assets/minecraft/textures/block/dirt.png")
                .exists()
        );
        let _ = fs::remove_dir_all(base);
    }
}
