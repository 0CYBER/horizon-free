//! A mod that alone stops a build (it hooks code the build's Horizon doesn't have, or declares
//! the same support module as another mod) is left out, with the mods that need it, and the
//! build runs again without them: the player still gets a game with the others, and is told
//! which mods were left out and why.
use super::Bundle;
use crate::i18n::trf;
use horizon_mod_package::dependencies::Dependency;
use std::collections::BTreeSet;

/// Why a mod was left out of a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// It doesn't apply to, or compile on, the Horizon the build uses.
    OtherVersion,
    /// It declares the same support module, in the same file, as this mod.
    Clash(String),
    /// It needs this mod, which was left out.
    Needs(String),
    /// It doesn't fit the build, or works on another build only: what the player reads about it.
    Unfit(String),
    /// It was judged to fit the build, and its compile against it failed.
    DidNotBuild,
}

/// A mod left out of a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    pub id: String,
    pub version: String,
    pub why: Why,
}

impl LeftOut {
    pub(super) fn new(bundle: &Bundle, why: Why) -> Self {
        Self {
            id: bundle.manifest.id.clone(),
            version: bundle.manifest.version.clone(),
            why,
        }
    }

    /// What the player reads.
    pub fn label(&self) -> String {
        match &self.why {
            Why::OtherVersion => trf(
                "{name} was left out: made for another Horizon version",
                &[("name", &self.id)],
            ),
            Why::Clash(other) => trf(
                "{name} was left out: clashes with {other}",
                &[("name", &self.id), ("other", other)],
            ),
            Why::Needs(other) => trf(
                "{name} was left out: it needs {other}, which was left out",
                &[("name", &self.id), ("other", other)],
            ),
            Why::Unfit(line) => trf(
                "{name} is switched off for this build: {why}",
                &[("name", &self.id), ("why", line)],
            ),
            Why::DidNotBuild => super::fit::did_not_build(&self.id),
        }
    }

    /// How a build's `native-mods.json` keeps it (`left_out`).
    pub(super) fn to_json(&self) -> serde_json::Value {
        let (why, with) = match &self.why {
            Why::OtherVersion => ("other-version", None),
            Why::Clash(other) => ("clash", Some(other)),
            Why::Needs(other) => ("needs", Some(other)),
            Why::Unfit(line) => ("unfit", Some(line)),
            Why::DidNotBuild => ("did-not-build", None),
        };
        serde_json::json!({"id": self.id, "version": self.version, "why": why, "with": with})
    }

    pub(super) fn from_json(value: &serde_json::Value) -> Option<Self> {
        let with = || value["with"].as_str().map(str::to_owned);
        Some(Self {
            id: value["id"].as_str()?.to_owned(),
            version: value["version"].as_str()?.to_owned(),
            why: match value["why"].as_str()? {
                "clash" => Why::Clash(with()?),
                "needs" => Why::Needs(with()?),
                "unfit" => Why::Unfit(with()?),
                "did-not-build" => Why::DidNotBuild,
                _ => Why::OtherVersion,
            },
        })
    }
}

/// `left` and, after them, every mod of `mods` that needs one of them (not optionally),
/// transitively, each once.
pub(super) fn with_dependents(left: &[LeftOut], mods: &[(&Bundle, &[Dependency])]) -> Vec<LeftOut> {
    let mut out = left.to_vec();
    let mut gone: BTreeSet<String> = left.iter().map(|l| l.id.clone()).collect();
    loop {
        let next = mods.iter().find_map(|(bundle, deps)| {
            let needed = deps.iter().find(|d| !d.optional && gone.contains(&d.id))?;
            (!gone.contains(&bundle.manifest.id))
                .then(|| LeftOut::new(bundle, Why::Needs(needed.id.clone())))
        });
        let Some(next) = next else {
            return out;
        };
        gone.insert(next.id.clone());
        out.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn bundle(root: &Path, id: &str, modules: &[(&str, &str)]) -> Bundle {
        let dir = root.join(id);
        super::super::tests::write_bundle(&dir, id, "fn hook() {}");
        let path = dir.join(super::super::NATIVE_MANIFEST);
        let mut text = std::fs::read_to_string(&path).unwrap();
        for (file, name) in modules {
            text += &format!(
                "[[native.module]]\nid = \"m\"\nfile = \"{file}\"\nname = \"{name}\"\nsource = \"src/m.rs\"\n"
            );
        }
        std::fs::write(&path, text).unwrap();
        std::fs::write(dir.join("src/m.rs"), "pub fn helper() {}").unwrap();
        super::super::bundle_in(&dir).unwrap().unwrap()
    }

    /// A helper that breaks a rule of the format is the mod's own source, not
    /// a Horizon it doesn't fit: the build fails with the tool's diagnostic
    /// (which names the injection and the code) rather than leaving the mod out
    /// as "made for another Horizon version" (#13971).

    #[test]
    fn the_mods_needing_a_left_out_mod_go_too_but_not_optional_ones() {
        let root = super::super::tests::dir("isolate-deps");
        let [lib, addon, extra, maybe, other] =
            ["lib", "addon", "extra", "maybe", "other"].map(|id| bundle(&root, id, &[]));
        let dep = |id: &str, optional| Dependency {
            id: id.into(),
            requirement: semver::VersionReq::STAR,
            optional,
            side: horizon_mod_package::manifest::Side::Both,
            reason: None,
            url: None,
            order: mod_vocab::DependencyOrder::After,
        };
        let (on_lib, on_addon, maybe_lib) = (
            [dep("lib", false)],
            [dep("addon", false)],
            [dep("lib", true)],
        );
        let mods: Vec<(&Bundle, &[Dependency])> = vec![
            (&extra, &on_addon),
            (&lib, &[]),
            (&addon, &on_lib),
            (&maybe, &maybe_lib),
            (&other, &[]),
        ];
        let left = with_dependents(&[LeftOut::new(&lib, Why::Unfit("why".into()))], &mods);
        let ids: Vec<&str> = left.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["lib", "addon", "extra"]);
        assert_eq!(
            left[2].label(),
            "extra was left out: it needs addon, which was left out"
        );
        // What a build's record keeps reads back the same.
        let did_not_build = LeftOut::new(&other, Why::DidNotBuild);
        for l in left.iter().chain([&did_not_build]) {
            assert_eq!(LeftOut::from_json(&l.to_json()).as_ref(), Some(l));
        }
        assert_eq!(left[0].label(), "lib is switched off for this build: why");
    }
}
