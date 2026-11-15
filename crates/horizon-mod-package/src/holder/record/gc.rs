//! Record groups are dead only while the permanent namespace gate is exclusive.
use super::*;
use crate::runtime_leases::cleanup::Cleanup;
use cap_fs_ext::DirExt;

fn retained_dir(dir: Dir) -> io::Result<Dir> {
    Ok(Dir::from_std_file(crate::runtime_leases::retained(
        dir.into_std_file(),
    )?))
}
fn regular(meta: &cap_std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    meta.is_file() && !meta.file_type().is_symlink() && meta.nlink() == 1
}
impl Cleanup {
    /// Collects recognized bootstrap payloads and their raw captures as complete groups.
    /// Unknown or malformed groups are retained. Never opens a shared registry or upgrades.
    pub fn collect_records(&self) -> io::Result<usize> {
        let records = match self.data.open_dir_nofollow("run-records") {
            Ok(dir) => retained_dir(dir)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut removed = 0;
        for candidate in records.entries()? {
            let name = candidate?.file_name();
            let Some(key) = name.to_str() else { continue };
            if key.len() != 64
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                continue;
            }
            let Ok(group) = records.open_dir_nofollow(&name) else {
                continue;
            };
            let group = retained_dir(group)?;
            #[cfg(windows)]
            {
                use cap_std::fs::MetadataExt;
                if group.dir_metadata()?.file_attributes() & 0x400 != 0 {
                    continue;
                }
            }
            let group_id = identity(&group)?;
            let mut files = Vec::new();
            let mut payload = None;
            let mut valid = true;
            for item in group.entries()? {
                let file_name = item?.file_name();
                if !matches!(
                    file_name.to_str(),
                    Some(
                        "launch.json"
                            | "stdout.log"
                            | "stdout.log.1"
                            | "stderr.log"
                            | "stderr.log.1"
                    )
                ) {
                    valid = false;
                    break;
                }
                let meta = group.symlink_metadata(&file_name)?;
                if !regular(&meta)
                    || (file_name != "launch.json" && meta.len() > crate::output::LIMIT)
                {
                    valid = false;
                    break;
                }
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let file = File::from_std(crate::runtime_leases::retained(
                    group.open_with(&file_name, &options)?.into_std(),
                )?);
                if token(&file)? != [meta.dev(), meta.ino()] || !regular(&file.metadata()?) {
                    valid = false;
                    break;
                }
                if file_name == "launch.json" {
                    let mut bytes = Vec::new();
                    (&file).take(LIMIT + 1).read_to_end(&mut bytes)?;
                    if bytes.len() as u64 <= LIMIT {
                        payload = serde_json::from_slice::<Record>(&bytes).ok();
                    }
                }
                files.push((file_name, file));
            }
            let Some(payload) = payload else { continue };
            if !valid
                || payload.entry.validate(16).is_err()
                || !payload.roots.iter().all(|root| {
                    payload
                        .entry
                        .trees
                        .iter()
                        .any(|tree| tree.identity == *root)
                })
                || open_directory(&payload.entry.data)
                    .ok()
                    .and_then(|dir| identity(&dir).ok())
                    != Some(identity(&self.data)?)
                || !payload.entry.trees.iter().any(|tree| {
                    tree.identity == group_id
                        && open_directory(&tree.path)
                            .ok()
                            .and_then(|dir| identity(&dir).ok())
                            == Some(group_id)
                })
            {
                continue;
            }
            // Retained handles prevent identity reuse. Recheck the entire group before unlinking.
            for (name, file) in &files {
                let meta = group.symlink_metadata(name)?;
                if !regular(&meta) || [meta.dev(), meta.ino()] != token(file)? {
                    valid = false;
                }
            }
            let current = retained_dir(records.open_dir_nofollow(&name)?)?;
            if !valid || identity(&current)? != group_id {
                continue;
            }
            for (name, _file) in &files {
                group.remove_file(name)?;
            }
            // Windows directory handles can deny deletion; the exclusive gate still excludes owners.
            // Arbitrary same-user namespace swaps are outside this protocol.
            drop(files);
            drop(current);
            drop(group);
            records.remove_dir(&name)?;
            removed += 1;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
