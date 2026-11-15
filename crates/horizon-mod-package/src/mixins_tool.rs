//! The horizon-mixins tool the launcher runs for `[[native.mixin]]` builds: the pinned source
//! ([`REV`]), the one change carried on it until upstream takes it ([`PATCH`], which narrows the
//! tool's "calls beneath a reference" rule from the whole function to the site a hook rewrites),
//! and the names of its prebuilt release. `.github/workflows/prebuilt-mixins-tool.yml` builds and
//! publishes it through `xtask mixins-tool`; the launcher downloads it, or builds the same
//! patched source where no prebuilt exists.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

/// The horizon-mixins commit the tool is built from.
pub const REV: &str = "e59da9094f0c2fdda6c627fe9eff22dec6a906f0";

pub const PATCH: &str = include_str!("../../../tools/mixins/narrow-reference-rule.patch");
pub const PATCH_NAME: &str = "tools/mixins/narrow-reference-rule.patch";

/// The file inside a prebuilt archive that says what it was built from.
pub const IDENTITY: &str = "identity.txt";

/// SHA-256 of [`PATCH`], in hex.
pub fn patch_hash() -> String {
    format!("{:x}", Sha256::digest(PATCH.as_bytes()))
}

/// The tool's release tag. The source and the patch decide the tool, so one release serves every
/// game build that pins them.
pub fn release_tag() -> String {
    format!("mixins-tool-{}-{}", &REV[..12], &patch_hash()[..12])
}

/// The prebuilt tool's archive for a rustc host target.
pub fn archive(target: &str) -> String {
    format!("horizon-mixins-{target}.tar.gz")
}

/// What [`IDENTITY`] starts with for `target`: the lines the launcher checks. The archive's file
/// may say more after them (the compiler that built it), which nothing checks.
pub fn identity(target: &str) -> String {
    format!("rev = {REV}\npatch = {}\ntarget = {target}\n", patch_hash())
}

/// Applies the unified diff `patch` to the files under `root`. Every hunk must find its old
/// lines exactly where the diff says; nothing is written unless every file's hunks apply.
pub fn apply(root: &Path, patch: &str) -> Result<(), String> {
    let mut changed = Vec::new();
    let mut lines = patch.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(path) = line.strip_prefix("+++ b/") else {
            continue;
        };
        let file = root.join(path);
        let text = fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?;
        let old: Vec<&str> = text.lines().collect();
        let mut new: Vec<String> = Vec::new();
        let mut at = 0;
        while let Some(header) = lines.next_if(|l| l.starts_with("@@ ")) {
            let start: usize = header
                .strip_prefix("@@ -")
                .and_then(|rest| rest.split([',', ' ']).next())
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("{path}: bad hunk header `{header}`"))?;
            let start = start.saturating_sub(1);
            if start < at {
                return Err(format!("{path}: hunks out of order at `{header}`"));
            }
            new.extend(old[at..start.min(old.len())].iter().map(|l| l.to_string()));
            at = start;
            while let Some(body) = lines.next_if(|l| {
                l.starts_with([' ', '-', '+']) && !l.starts_with("--- ") && !l.starts_with("+++ ")
                    || l.is_empty()
            }) {
                let (mark, rest) = body.split_at(body.len().min(1));
                match mark {
                    "+" => new.push(rest.to_owned()),
                    // An empty line in a diff is an unchanged empty line.
                    " " | "" | "-" => {
                        if old.get(at) != Some(&rest) {
                            return Err(format!("{path}: line {} differs at `{header}`", at + 1));
                        }
                        at += 1;
                        if mark != "-" {
                            new.push(rest.to_owned());
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        new.extend(old[at..].iter().map(|l| l.to_string()));
        let mut out = new.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        changed.push((file, out));
    }
    if changed.is_empty() {
        return Err("the patch changes no file".into());
    }
    for (file, text) in changed {
        fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -2,3 +2,4 @@ fn a()
 two
-three
+three, narrowed
+three and a half
 four
";

    fn tree(text: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), text).unwrap();
        dir
    }

    #[test]
    fn a_hunk_applies_where_its_old_lines_are() {
        let dir = tree("one\ntwo\nthree\nfour\nfive\n");
        apply(dir.path(), DIFF).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "one\ntwo\nthree, narrowed\nthree and a half\nfour\nfive\n"
        );
    }

    #[test]
    fn changed_source_is_refused_and_left_alone() {
        let dir = tree("one\ntwo\nTHREE\nfour\n");
        let why = apply(dir.path(), DIFF).unwrap_err();
        assert!(why.contains("line 3 differs"), "{why}");
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "one\ntwo\nTHREE\nfour\n"
        );
    }
}
