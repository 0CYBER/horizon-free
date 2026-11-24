use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(crate) struct Resolved {
    pub(crate) commit: String,
    pub(crate) watch_paths: Vec<PathBuf>,
}

pub(crate) fn resolve(
    env_id: Option<&str>,
    manifest_dir: &Path,
    git_program: &OsStr,
) -> Result<Resolved, String> {
    if let Some(raw) = env_id {
        return resolve_from_env(raw);
    }

    let git_error = match Command::new(git_program)
        .args(["rev-parse", "HEAD"])
        .current_dir(manifest_dir)
        .output()
    {
        Ok(output) if output.status.success() => {
            let commit = String::from_utf8(output.stdout)
                .map_err(|error| format!("git rev-parse HEAD returned non-UTF-8: {error}"))?
                .trim()
                .to_owned();
            if commit.is_empty() {
                "git rev-parse HEAD returned an empty commit".to_owned()
            } else {
                let watch_paths = read_git_head(manifest_dir)
                    .map(|resolved| resolved.watch_paths)
                    .unwrap_or_default();
                return Ok(Resolved {
                    commit,
                    watch_paths,
                });
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!(
                "git rev-parse HEAD exited with {} ({})",
                output.status,
                stderr.trim()
            )
        }
        Err(error) => format!("failed to run git: {error}"),
    };

    resolve_without_git(manifest_dir, &git_error)
}

/// `HORIZON_GAME_BUILD_ID` wins over any repository.
fn resolve_from_env(raw: &str) -> Result<Resolved, String> {
    let commit = raw.trim().to_owned();
    if commit.is_empty() {
        return Err("game build identity must not be empty".to_owned());
    }
    Ok(Resolved {
        commit,
        watch_paths: Vec::new(),
    })
}

/// The `.git` fallback when `git` itself could not answer; starts no process.
fn resolve_without_git(manifest_dir: &Path, git_error: &str) -> Result<Resolved, String> {
    read_git_head(manifest_dir).map_err(|git_dir_error| {
        format!(
            "{git_error}; fallback reading .git also failed ({git_dir_error}); \
             set HORIZON_GAME_BUILD_ID for staged builds or source archives without .git"
        )
    })
}

fn read_git_head(start: &Path) -> Result<Resolved, String> {
    let git_dir = locate_git_dir(start)?;
    let common_dir = resolve_common_dir(&git_dir)?;
    let head_path = git_dir.join("HEAD");
    let mut watch_paths = vec![head_path.clone()];
    let mut content = fs::read_to_string(&head_path)
        .map_err(|error| format!("read {}: {error}", head_path.display()))?;

    for _ in 0..8 {
        let trimmed = content.trim();
        let Some(rest) = trimmed.strip_prefix("ref:") else {
            validate_oid(trimmed)?;
            return Ok(Resolved {
                commit: trimmed.to_owned(),
                watch_paths,
            });
        };

        let ref_name = rest.trim();
        let rel_ref = validate_rel_ref(ref_name)?;
        if let Some(loose_path) = find_loose_ref(&git_dir, &common_dir, &rel_ref) {
            content = fs::read_to_string(&loose_path)
                .map_err(|error| format!("read {}: {error}", loose_path.display()))?;
            watch_paths.push(loose_path);
            continue;
        }

        if let Some((oid, packed_path)) = find_packed_ref(&git_dir, &common_dir, ref_name)? {
            watch_paths.push(packed_path);
            for root in [&git_dir, &common_dir] {
                if let Some(parent) = nearest_existing_dir(root, &root.join(&rel_ref))
                    && !watch_paths.contains(&parent)
                {
                    watch_paths.push(parent);
                }
            }
            return Ok(Resolved {
                commit: oid,
                watch_paths,
            });
        }

        return Err(format!(
            "unresolved git ref {ref_name:?} under {}",
            git_dir.display()
        ));
    }

    Err(format!(
        "symbolic ref chain in {} exceeded 8 hops",
        head_path.display()
    ))
}

fn locate_git_dir(start: &Path) -> Result<PathBuf, String> {
    for dir in start.ancestors() {
        let candidate = dir.join(".git");
        let Ok(meta) = fs::metadata(&candidate) else {
            continue;
        };
        if meta.is_dir() {
            return Ok(candidate);
        }
        if meta.is_file() {
            let text = fs::read_to_string(&candidate)
                .map_err(|error| format!("read {}: {error}", candidate.display()))?;
            let target = text
                .trim()
                .strip_prefix("gitdir:")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("invalid gitdir file {}", candidate.display()))?;
            return Ok(resolve_relative(dir, target));
        }
    }
    Err(format!("no .git found above {}", start.display()))
}

fn resolve_common_dir(git_dir: &Path) -> Result<PathBuf, String> {
    let commondir_path = git_dir.join("commondir");
    if !commondir_path.is_file() {
        return Ok(git_dir.to_owned());
    }
    let text = fs::read_to_string(&commondir_path)
        .map_err(|error| format!("read {}: {error}", commondir_path.display()))?;
    let target = text.trim();
    if target.is_empty() {
        return Err(format!("empty commondir file {}", commondir_path.display()));
    }
    Ok(resolve_relative(git_dir, target))
}

fn resolve_relative(base: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut parts: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(parts.last(), Some(Component::Normal(_))) {
                    parts.pop();
                } else {
                    parts.push(component);
                }
            }
            _ => parts.push(component),
        }
    }
    parts.into_iter().collect()
}

fn validate_rel_ref(ref_name: &str) -> Result<PathBuf, String> {
    if ref_name.is_empty() {
        return Err("empty git ref name".to_owned());
    }
    let mut relative = PathBuf::new();
    for component in Path::new(ref_name).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            _ => return Err(format!("unsafe git ref path {ref_name:?}")),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(format!("empty git ref path {ref_name:?}"));
    }
    Ok(relative)
}

fn find_loose_ref(git_dir: &Path, common_dir: &Path, rel_ref: &Path) -> Option<PathBuf> {
    let primary = git_dir.join(rel_ref);
    if primary.is_file() {
        return Some(primary);
    }
    if common_dir != git_dir {
        let shared = common_dir.join(rel_ref);
        if shared.is_file() {
            return Some(shared);
        }
    }
    None
}

fn find_packed_ref(
    git_dir: &Path,
    common_dir: &Path,
    ref_name: &str,
) -> Result<Option<(String, PathBuf)>, String> {
    let mut candidates = vec![common_dir.join("packed-refs")];
    if git_dir != common_dir {
        candidates.push(git_dir.join("packed-refs"));
    }
    for packed_path in candidates {
        if !packed_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&packed_path)
            .map_err(|error| format!("read {}: {error}", packed_path.display()))?;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('^') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            if let (Some(oid), Some(name), None) = (parts.next(), parts.next(), parts.next())
                && name == ref_name
            {
                validate_oid(oid)?;
                return Ok(Some((oid.to_owned(), packed_path)));
            }
        }
    }
    Ok(None)
}

/// The closest existing directory above `target`, never above `<root>/refs`: watching the git
/// dir itself would rerun the build script on every index or log write.
fn nearest_existing_dir(root: &Path, target: &Path) -> Option<PathBuf> {
    let floor = root.join("refs");
    let mut current = target.parent();
    while let Some(dir) = current {
        if !dir.starts_with(&floor) {
            break;
        }
        if dir.is_dir() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn validate_oid(oid: &str) -> Result<(), String> {
    let valid_len = oid.len() == 40 || oid.len() == 64;
    if valid_len && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("invalid git object id {oid:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "horizon-build-id-{prefix}-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn resolve_from_dot_git(dir: &Path) -> Result<Resolved, String> {
        resolve_without_git(dir, "failed to run git")
    }

    #[test]
    fn env_override_wins_without_git_or_repo() {
        let resolved = resolve_from_env("  custom-build-123  ").unwrap();
        assert_eq!(resolved.commit, "custom-build-123");
        assert!(resolved.watch_paths.is_empty());
    }

    #[test]
    fn empty_env_override_is_rejected() {
        let err = resolve_from_env("   ").err().unwrap();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn reads_loose_ref_without_git_from_nested_crate_dir() {
        let dir = TempDir::new("loose");
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let git_dir = dir.path().join(".git");
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let ref_file = git_dir.join("refs/heads/main");
        fs::write(&ref_file, format!("{sha1}\n")).unwrap();
        let crate_dir = dir.path().join("crates/horizon-mod-host");
        fs::create_dir_all(&crate_dir).unwrap();

        let resolved = resolve_from_dot_git(&crate_dir).unwrap();
        assert_eq!(resolved.commit, sha1);
        assert!(resolved.watch_paths.contains(&git_dir.join("HEAD")));
        assert!(resolved.watch_paths.contains(&ref_file));
    }

    #[test]
    fn reads_packed_refs_without_git() {
        let dir = TempDir::new("packed");
        let sha1 = "abcdef0123456789abcdef0123456789abcdef01";
        let other = "1111111111111111111111111111111111111111";
        let git_dir = dir.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let packed = git_dir.join("packed-refs");
        fs::write(
            &packed,
            format!(
                "# pack-refs with: peeled fully-peeled sorted\n\
                 {other} refs/tags/v1.0\n\
                 ^{other}\n\
                 {sha1} refs/heads/main\n"
            ),
        )
        .unwrap();

        let resolved = resolve_from_dot_git(dir.path()).unwrap();
        assert_eq!(resolved.commit, sha1);
        assert!(resolved.watch_paths.contains(&packed));
    }

    #[test]
    fn reads_detached_head_sha1_and_sha256_without_git() {
        let sha1 = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let dir1 = TempDir::new("detached1");
        fs::create_dir_all(dir1.path().join(".git")).unwrap();
        fs::write(dir1.path().join(".git/HEAD"), format!("{sha1}\n")).unwrap();
        assert_eq!(resolve_from_dot_git(dir1.path()).unwrap().commit, sha1);

        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let dir2 = TempDir::new("detached256");
        fs::create_dir_all(dir2.path().join(".git")).unwrap();
        fs::write(dir2.path().join(".git/HEAD"), format!("{sha256}\n")).unwrap();
        assert_eq!(resolve_from_dot_git(dir2.path()).unwrap().commit, sha256);
    }

    #[test]
    fn reads_worktree_gitfile_and_commondir_without_git() {
        let dir = TempDir::new("worktree");
        let common = dir.path().join("main/.git");
        let wt_git = common.join("worktrees/feature");
        let wt_root = dir.path().join("feature-wt");
        let crate_dir = wt_root.join("crates/horizon-mod-host");
        fs::create_dir_all(common.join("refs/heads")).unwrap();
        fs::create_dir_all(&wt_git).unwrap();
        fs::create_dir_all(&crate_dir).unwrap();

        let sha1 = "2222222222222222222222222222222222222222";
        fs::write(common.join("refs/heads/wt-branch"), format!("{sha1}\n")).unwrap();
        fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        fs::write(wt_git.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        fs::write(
            wt_root.join(".git"),
            "gitdir: ../main/.git/worktrees/feature\n",
        )
        .unwrap();

        let resolved = resolve_from_dot_git(&crate_dir).unwrap();
        assert_eq!(resolved.commit, sha1);
    }

    #[test]
    fn rejects_traversal_ref_and_invalid_oid() {
        let dir = TempDir::new("traversal");
        let git_dir = dir.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: ../outside\n").unwrap();
        let err = resolve_from_dot_git(dir.path()).err().unwrap();
        assert!(err.contains("unsafe git ref"), "{err}");

        fs::write(git_dir.join("HEAD"), "not-a-sha\n").unwrap();
        let err = resolve_from_dot_git(dir.path()).err().unwrap();
        assert!(err.contains("invalid git object id"), "{err}");
    }

    #[test]
    fn reports_actionable_error_when_git_and_dot_git_are_both_missing() {
        let dir = TempDir::new("no-git");
        let err = resolve_from_dot_git(dir.path()).err().unwrap();
        assert!(err.contains("HORIZON_GAME_BUILD_ID"), "{err}");
    }

    #[test]
    fn packed_ref_watches_loose_ref_parent_and_detects_loose_override() {
        let dir = TempDir::new("packed-loose");
        let packed_oid = "abcdef0123456789abcdef0123456789abcdef01";
        let loose_oid = "9999999999999999999999999999999999999999";
        let git_dir = dir.path().join(".git");
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/topic/main\n").unwrap();
        let packed = git_dir.join("packed-refs");
        fs::write(&packed, format!("{packed_oid} refs/heads/topic/main\n")).unwrap();

        let initial = resolve_from_dot_git(dir.path()).unwrap();
        assert_eq!(initial.commit, packed_oid);
        assert!(initial.watch_paths.contains(&packed));
        assert!(initial.watch_paths.contains(&git_dir.join("refs/heads")));

        let loose_dir = git_dir.join("refs/heads/topic");
        fs::create_dir_all(&loose_dir).unwrap();
        let loose_ref = loose_dir.join("main");
        fs::write(&loose_ref, format!("{loose_oid}\n")).unwrap();

        let updated = resolve_from_dot_git(dir.path()).unwrap();
        assert_eq!(updated.commit, loose_oid);
        assert!(updated.watch_paths.contains(&loose_ref));
    }

    #[test]
    fn packed_worktree_ref_watches_shared_refs_parent() {
        let dir = TempDir::new("packed-wt");
        let packed_oid = "3333333333333333333333333333333333333333";
        let loose_oid = "4444444444444444444444444444444444444444";
        let common = dir.path().join("main/.git");
        let wt_git = common.join("worktrees/feature");
        let wt_root = dir.path().join("feature-wt");
        let crate_dir = wt_root.join("crates/horizon-mod-host");
        fs::create_dir_all(common.join("refs/heads")).unwrap();
        fs::create_dir_all(&wt_git).unwrap();
        fs::create_dir_all(&crate_dir).unwrap();

        fs::write(
            common.join("packed-refs"),
            format!("{packed_oid} refs/heads/wt-packed\n"),
        )
        .unwrap();
        fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        fs::write(wt_git.join("HEAD"), "ref: refs/heads/wt-packed\n").unwrap();
        fs::write(
            wt_root.join(".git"),
            "gitdir: ../main/.git/worktrees/feature\n",
        )
        .unwrap();

        let initial = resolve_from_dot_git(&crate_dir).unwrap();
        assert_eq!(initial.commit, packed_oid);
        assert!(initial.watch_paths.contains(&common.join("refs/heads")));

        let loose_ref = common.join("refs/heads/wt-packed");
        fs::write(&loose_ref, format!("{loose_oid}\n")).unwrap();
        let updated = resolve_from_dot_git(&crate_dir).unwrap();
        assert_eq!(updated.commit, loose_oid);
        assert!(updated.watch_paths.contains(&loose_ref));
    }
}
