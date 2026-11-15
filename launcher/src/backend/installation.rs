//! Installation publication excludes concurrent capability snapshots.
use super::*;

pub(crate) fn install_as(
    release: &Release,
    tag: &str,
    sha256: &str,
    requested_architecture: Option<GameArchitecture>,
    progress: impl Fn(f32),
) -> Result<(), String> {
    let _publication =
        crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    let (url, size) = release
        .asset
        .clone()
        .ok_or_else(|| format!("{tag} has no build for this platform."))?;
    let root = versions_dir();
    let partial = root.join(format!(".{tag}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let archive = partial.join("download");
    download(&url, size, sha256, &archive, &progress)?;
    let unpacked = partial.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    progress(0.97);
    // The archives hold one top folder (horizon-linux-x86_64/...); the version folder is its contents.
    let entries: Vec<_> = fs::read_dir(&unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let top = match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => unpacked.clone(),
    };
    if !top.join(CLIENT).exists() {
        return Err(format!("The download has no {CLIENT}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(top.join(CLIENT), fs::Permissions::from_mode(0o755));
    }
    if let (Some(requested), Some(binary)) = (requested_architecture, architecture_for_asset(&url))
    {
        fs::write(
            top.join(GAME_BUILD_ARCHITECTURE),
            serde_json::to_vec(&InstalledGameArchitecture { requested, binary })
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    let target = version_dir(tag);
    let _ = fs::remove_dir_all(&target);
    fs::rename(&top, &target).map_err(|e| e.to_string())?;
    crate::run_leases::publish(&target).map_err(crate::native_mods::cache_error)?;
    fs::write(target.join(COMPLETE), "").map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    reuse_prepared_for(tag);
    progress(1.0);
    Ok(())
}
